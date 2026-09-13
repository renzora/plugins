//! Panel state, the background workers that fill it, and the on-disk catalogue
//! cache.
//!
//! # Why every request runs on its own thread
//!
//! [`renzora::net::Request::send`] blocks *its own thread* while the frame
//! carries on: it queues the request and parks until the per-frame pump hands
//! the answer back. Calling it from a system therefore waits for a frame that
//! cannot advance until the system returns, and the contract crate reports that
//! as [`renzora::net::Error::NoPump`] rather than hanging. So nothing in this
//! module touches the network from a system — the systems start threads and
//! drain [`Inbox`], and the threads do the blocking.
//!
//! That also settles the shape of the results channel. A `std::sync::mpsc`
//! `Sender`/`Receiver` pair is `Send` but not `Sync`, and a Bevy resource must be
//! both, so the landing zone is an `Arc<Mutex<Vec<_>>>` — the same arrangement
//! `renzora_scripting`'s `HttpInbox` uses, and it needs no dependency to say it.

use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use bevy::prelude::*;

use crate::api::{self, Asset, Kind};

/// How long a cached catalogue is trusted before it is fetched again.
///
/// Poly Haven publishes a handful of assets a week, so a day-old list is missing
/// almost nothing; the cost of being wrong is that a brand new asset is not
/// listed until tomorrow, against a 1.5 MB download every time the panel opens.
const CACHE_TTL: Duration = Duration::from_secs(24 * 60 * 60);

/// Cap on one downloaded file.
///
/// Enforced by the backend as the body arrives rather than after, so it bounds
/// memory rather than merely reporting on it. 512 MB clears the largest thing
/// this plugin ever asks for by a wide margin (a 24k EXR is ~400 MB) while still
/// refusing a response that has no business being this size.
const MAX_FILE_BYTES: u32 = 512 * 1024 * 1024;

/// Cap on a catalogue response. The largest is ~2 MB.
const MAX_CATALOGUE_BYTES: u32 = 32 * 1024 * 1024;

/// What a worker thread sends back.
pub enum Msg {
    Catalogue {
        kind: Kind,
        result: Result<Vec<Asset>, String>,
    },
    /// One file of a download finished; `done` counts completed files.
    Progress {
        slug: String,
        done: usize,
        total: usize,
    },
    Finished {
        slug: String,
        /// The project-relative folder the files landed in, or why they did not.
        result: Result<String, String>,
    },
}

/// Shared landing zone: worker threads push, [`crate::drain_inbox`] empties it
/// once a frame.
#[derive(Clone, Default)]
pub struct Inbox(Arc<Mutex<Vec<Msg>>>);

impl Inbox {
    fn push(&self, msg: Msg) {
        // A poisoned mutex means a worker panicked while holding it. Dropping the
        // message is the right response: the alternative is propagating that
        // panic into the frame loop and taking the editor down over a failed
        // thumbnail fetch.
        if let Ok(mut queue) = self.0.lock() {
            queue.push(msg);
        }
    }

    pub fn drain(&self) -> Vec<Msg> {
        self.0
            .lock()
            .map(|mut queue| std::mem::take(&mut *queue))
            .unwrap_or_default()
    }
}

/// One kind's catalogue, as the panel sees it.
#[derive(Default)]
pub enum Catalogue {
    /// Never asked for. The panel starts here and leaves it on first open, so
    /// merely having the plugin installed makes no requests.
    #[default]
    Idle,
    Loading,
    Ready(Vec<Asset>),
    Failed(String),
}

/// A download in progress, or one that has finished.
pub struct Job {
    /// Identifies the job, and is how a tile finds its own: the grid looks the
    /// job up by slug rather than the job carrying anything to display. There is
    /// no name here for that reason — the tile already has one, from the
    /// catalogue entry it was built from.
    pub slug: String,
    pub kind: Kind,
    /// Absolute folder the files were written to. Kept because a finished model
    /// download has to be handed to the import pipeline by path, and the pick
    /// that produced it is gone by then.
    pub folder: PathBuf,
    pub done: usize,
    pub total: usize,
    /// `None` while running; `Some(Ok(folder))` or `Some(Err(why))` when over.
    pub outcome: Option<Result<String, String>>,
    /// Set once the folder has been queued for import, so the handoff happens
    /// exactly once however many frames the job stays on screen.
    pub handed_off: bool,
}

impl Job {
    pub fn running(&self) -> bool {
        self.outcome.is_none()
    }
}

/// Everything the panel reads and the systems write.
#[derive(Resource)]
pub struct Hub {
    /// Which tab is showing.
    pub kind: Kind,
    /// The resolution downloads ask for. Not every asset publishes it; the plan
    /// falls back to the nearest published size.
    pub res: &'static str,
    /// Lowercased search terms, refreshed from the search box each frame.
    pub terms: Vec<String>,
    /// Which [`PAGE_SIZE`] window of the filtered list the grid is showing.
    pub page_index: usize,
    /// Indexed by [`Kind::index`].
    pub catalogues: [Catalogue; 3],
    /// Newest first, capped — this is a status readout, not a history.
    pub jobs: Vec<Job>,
    pub inbox: Inbox,
}

impl Default for Hub {
    fn default() -> Self {
        Self {
            kind: Kind::Models,
            res: "2k",
            terms: Vec::new(),
            page_index: 0,
            catalogues: Default::default(),
            jobs: Vec::new(),
            inbox: Inbox::default(),
        }
    }
}

/// How many finished jobs stay on screen.
const JOB_HISTORY: usize = 6;

/// How many tiles the grid builds at once — one page.
///
/// The grid is not virtualised, so every entry costs six UI entities and a
/// thumbnail slot. The textures catalogue is about a thousand entries, which
/// would be six thousand entities and a thousand queued downloads for a wall of
/// tiles nobody scrolls to the end of.
///
/// This was a hard cap on the first 150 matches, with the search box as the only
/// way to reach anything past them. That is fine for "find the brick wall" and
/// useless for browsing, which is what a library like this is mostly for — so it
/// pages instead, and the cost per page is the same flat number it was.
pub const PAGE_SIZE: usize = 150;

impl Hub {
    pub fn catalogue(&self) -> &Catalogue {
        &self.catalogues[self.kind.index()]
    }

    /// The current tab's entries that match the search box, in catalogue order.
    ///
    /// Borrowed rather than cloned: this runs inside a `keyed_list` snapshot,
    /// which is the hottest closure in the panel, and the largest catalogue is
    /// about a thousand entries.
    pub fn visible(&self) -> Vec<&Asset> {
        let Catalogue::Ready(assets) = self.catalogue() else {
            return Vec::new();
        };
        if self.terms.is_empty() {
            return assets.iter().collect();
        }
        assets.iter().filter(|a| a.matches(&self.terms)).collect()
    }

    /// The entries the grid actually builds: one [`PAGE_SIZE`] window of
    /// [`visible`](Self::visible).
    ///
    /// The window is clamped rather than trusted. `page_index` is reset when the
    /// search or the tab changes, but a catalogue can also arrive *after* a page
    /// was turned, and an unclamped `skip` past the end would silently show an
    /// empty grid on a tab that has results.
    pub fn page(&self) -> Vec<&Asset> {
        let visible = self.visible();
        // Clamped to the last page's own start, not to the last item: clamping
        // to the item would land mid-page and show a partial window that no
        // page number describes.
        let last_start = (self.page_count() - 1) * PAGE_SIZE;
        let start = (self.page_index * PAGE_SIZE).min(last_start);
        visible.into_iter().skip(start).take(PAGE_SIZE).collect()
    }

    /// How many pages the current filter produces. Always at least one, so
    /// "page 1 of 1" is what an empty result says rather than "page 1 of 0".
    pub fn page_count(&self) -> usize {
        self.visible().len().div_ceil(PAGE_SIZE).max(1)
    }

    /// Move by `delta` pages, stopping at either end.
    pub fn turn_page(&mut self, delta: i32) {
        let last = self.page_count() - 1;
        let next = (self.page_index as i32 + delta).clamp(0, last as i32) as usize;
        if next != self.page_index {
            self.page_index = next;
        }
    }

    /// Back to the first page. Called whenever the result set changes underneath
    /// the reader — a new search or a new tab — because staying on page 5 of a
    /// list that now has two is how a filter looks like it returned nothing.
    pub fn reset_page(&mut self) {
        if self.page_index != 0 {
            self.page_index = 0;
        }
    }

    pub fn job(&self, slug: &str) -> Option<&Job> {
        self.jobs.iter().find(|j| j.slug == slug)
    }

    /// Fold one worker message into the state.
    pub fn apply(&mut self, msg: Msg) {
        match msg {
            Msg::Catalogue { kind, result } => {
                self.catalogues[kind.index()] = match result {
                    Ok(assets) => Catalogue::Ready(assets),
                    Err(why) => Catalogue::Failed(why),
                };
            }
            Msg::Progress { slug, done, total } => {
                if let Some(job) = self.jobs.iter_mut().find(|j| j.slug == slug) {
                    job.done = done;
                    job.total = total;
                }
            }
            Msg::Finished { slug, result } => {
                if let Some(job) = self.jobs.iter_mut().find(|j| j.slug == slug) {
                    if let Ok(folder) = &result {
                        info!("[polyhaven] {slug} -> {folder}");
                    }
                    if let Err(why) = &result {
                        warn!("[polyhaven] {slug} failed: {why}");
                    }
                    job.outcome = Some(result);
                }
            }
        }
        // Trim finished jobs from the back, keeping every running one however
        // many there are — a job still in flight is the thing the user most
        // needs to see, and dropping it from the list would look like it stopped.
        while self.jobs.len() > JOB_HISTORY {
            let Some(oldest) = self.jobs.iter().rposition(|j| !j.running()) else {
                break;
            };
            self.jobs.remove(oldest);
        }
    }
}

// ============================================================================
// Catalogue
// ============================================================================

/// Where a fetched catalogue is cached between sessions.
///
/// The system temp directory rather than the project: the catalogue describes
/// Poly Haven, not this project, so caching it per-project would refetch the
/// same 1.5 MB for every project on the machine and leave a stray file in each
/// one. Being wiped on reboot is fine — the TTL already assumes it can vanish.
pub fn cache_path(kind: Kind) -> PathBuf {
    std::env::temp_dir()
        .join("renzora-polyhaven")
        .join(format!("{}.json", kind.slug()))
}

/// Is a cache file present and younger than [`CACHE_TTL`]?
///
/// Uses the file's own mtime rather than a timestamp written inside it, so the
/// cache stays a plain copy of the response that can be read, diffed or deleted
/// without this plugin's help.
fn cache_is_fresh(path: &Path) -> bool {
    let Ok(modified) = std::fs::metadata(path).and_then(|m| m.modified()) else {
        return false;
    };
    // A clock that moved backwards makes `elapsed` fail. Treat that as stale:
    // refetching costs one request, trusting an unknown age could pin a bad list
    // in place indefinitely.
    modified.elapsed().map(|age| age < CACHE_TTL).unwrap_or(false)
}

/// Load one kind's catalogue, from cache when it is fresh and from the API when
/// it is not, and post the result.
pub fn spawn_catalogue_fetch(kind: Kind, inbox: Inbox) {
    std::thread::spawn(move || {
        let path = cache_path(kind);
        if cache_is_fresh(&path) {
            if let Ok(text) = std::fs::read_to_string(&path) {
                // A cache that will not parse is not an error worth reporting:
                // it is a truncated write from a previous session, and the
                // request below replaces it.
                if let Ok(assets) = api::parse_catalogue(&text) {
                    inbox.push(Msg::Catalogue {
                        kind,
                        result: Ok(assets),
                    });
                    return;
                }
            }
        }

        let result = fetch_text(&api::catalogue_url(kind), MAX_CATALOGUE_BYTES)
            .and_then(|text| {
                let assets = api::parse_catalogue(&text)?;
                // Written only after it parsed, so a malformed response never
                // becomes tomorrow's cache. Best-effort: an unwritable temp
                // directory costs a refetch next session, not this one.
                if let Some(parent) = path.parent() {
                    let _ = std::fs::create_dir_all(parent);
                }
                let _ = std::fs::write(&path, &text);
                Ok(assets)
            });
        inbox.push(Msg::Catalogue { kind, result });
    });
}

// ============================================================================
// Download
// ============================================================================

/// Fetch one asset's files into `folder`.
///
/// `folder` is absolute and already includes the per-asset directory: it comes
/// from the destination overlay, resolved on the main thread while it still has
/// `CurrentProject` and `FolderPick`. Passing the finished path in is what keeps
/// this function free of the World, which a thread cannot reach.
///
/// `label` is the same folder written project-relative, for the progress chip.
/// Computed by the caller for the same reason.
pub fn spawn_download(
    kind: Kind,
    res: &'static str,
    slug: String,
    folder: PathBuf,
    label: String,
    inbox: Inbox,
) {
    std::thread::spawn(move || {
        let result = download(kind, res, &slug, &folder, &label, &inbox);
        inbox.push(Msg::Finished { slug, result });
    });
}

/// The body of [`spawn_download`], written to return `Result` so every failure
/// path reports rather than logging and vanishing.
fn download(
    kind: Kind,
    res: &'static str,
    slug: &str,
    folder: &Path,
    label: &str,
    inbox: &Inbox,
) -> Result<String, String> {
    let listing = fetch_text(&api::files_url(slug), MAX_CATALOGUE_BYTES)?;
    let files: serde_json::Value =
        serde_json::from_str(&listing).map_err(|e| format!("file list is not valid JSON: {e}"))?;
    let plan = api::plan(kind, &files, res)?;

    std::fs::create_dir_all(folder).map_err(|e| format!("{}: {e}", folder.display()))?;

    let total = plan.len();
    inbox.push(Msg::Progress {
        slug: slug.to_string(),
        done: 0,
        total,
    });

    for (done, file) in plan.iter().enumerate() {
        let bytes = fetch_bytes(&file.url, MAX_FILE_BYTES)?;
        let target = folder.join(&file.rel);
        // A glTF's `include` keys carry subdirectories (`textures/…`), which do
        // not exist yet. Already validated by `api::safe_rel`, so this cannot
        // create a directory outside `folder`.
        if let Some(parent) = target.parent() {
            std::fs::create_dir_all(parent).map_err(|e| format!("{}: {e}", parent.display()))?;
        }
        std::fs::write(&target, &bytes).map_err(|e| format!("{}: {e}", target.display()))?;
        inbox.push(Msg::Progress {
            slug: slug.to_string(),
            done: done + 1,
            total,
        });
    }

    Ok(label.to_string())
}

// ============================================================================
// Transport
// ============================================================================

/// GET `url`, returning the body as bytes.
///
/// Blocking, so this only ever runs on a worker thread — see the module docs.
fn fetch_bytes(url: &str, max: u32) -> Result<Vec<u8>, String> {
    let response = renzora::net::Request::get(url)
        .header("User-Agent", api::USER_AGENT)
        .max_bytes(max)
        .send()
        .map_err(|e| e.to_string())?;
    // A status is not a transport error — `renzora::net` reports 4xx as a
    // successful request precisely so a caller can read the body an API sends
    // with its error. Here there is nothing useful in it, so it becomes one.
    if !response.is_ok() {
        return Err(format!("HTTP {} from {url}", response.status));
    }
    Ok(response.body)
}

/// [`fetch_bytes`] for the two JSON endpoints.
fn fetch_text(url: &str, max: u32) -> Result<String, String> {
    let bytes = fetch_bytes(url, max)?;
    String::from_utf8(bytes).map_err(|_| format!("response from {url} is not UTF-8"))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn finished_jobs_are_trimmed_and_running_ones_are_not() {
        let mut hub = Hub::default();
        for i in 0..JOB_HISTORY + 3 {
            hub.jobs.push(Job {
                slug: format!("s{i}"),
                kind: Kind::Models,
                folder: PathBuf::new(),
                done: 0,
                total: 1,
                // Everything finished except the newest, which is still going.
                outcome: if i == 0 { None } else { Some(Ok(String::new())) },
                handed_off: false,
            });
        }
        hub.apply(Msg::Progress {
            slug: "s0".into(),
            done: 1,
            total: 1,
        });
        assert_eq!(hub.jobs.len(), JOB_HISTORY);
        assert!(hub.jobs.iter().any(|j| j.slug == "s0"), "the running job survives");
    }

    #[test]
    fn a_catalogue_result_lands_on_its_own_tab() {
        let mut hub = Hub::default();
        hub.apply(Msg::Catalogue {
            kind: Kind::Hdris,
            result: Ok(vec![Asset {
                slug: "a".into(),
                name: "A".into(),
                ..Default::default()
            }]),
        });
        assert!(matches!(hub.catalogues[Kind::Hdris.index()], Catalogue::Ready(_)));
        assert!(matches!(hub.catalogues[Kind::Models.index()], Catalogue::Idle));
    }

    /// A hub holding `n` models named `a0`, `a1`, …
    fn hub_of(n: usize) -> Hub {
        let mut hub = Hub::default();
        hub.kind = Kind::Models;
        hub.catalogues[Kind::Models.index()] = Catalogue::Ready(
            (0..n)
                .map(|i| Asset {
                    slug: format!("a{i}"),
                    name: format!("a{i}"),
                    ..Default::default()
                })
                .collect(),
        );
        hub
    }

    #[test]
    fn page_count_is_never_zero() {
        assert_eq!(hub_of(0).page_count(), 1, "empty is page 1 of 1, not 1 of 0");
        assert_eq!(hub_of(1).page_count(), 1);
        assert_eq!(hub_of(PAGE_SIZE).page_count(), 1, "an exact fit is one page");
        assert_eq!(hub_of(PAGE_SIZE + 1).page_count(), 2);
    }

    #[test]
    fn paging_stops_at_both_ends() {
        let mut hub = hub_of(PAGE_SIZE * 2 + 1);
        assert_eq!(hub.page_count(), 3);

        hub.turn_page(-1);
        assert_eq!(hub.page_index, 0, "already at the first page");

        hub.turn_page(1);
        hub.turn_page(1);
        hub.turn_page(1);
        assert_eq!(hub.page_index, 2, "clamped to the last page");
    }

    #[test]
    fn each_page_is_its_own_window() {
        let mut hub = hub_of(PAGE_SIZE + 5);
        let first: Vec<String> = hub.page().iter().map(|a| a.slug.clone()).collect();
        assert_eq!(first.len(), PAGE_SIZE);

        hub.turn_page(1);
        let second: Vec<String> = hub.page().iter().map(|a| a.slug.clone()).collect();
        assert_eq!(second.len(), 5, "the last page is a partial one");
        assert!(
            second.iter().all(|s| !first.contains(s)),
            "pages must not overlap"
        );
    }

    #[test]
    fn a_stale_page_index_clamps_to_a_real_page() {
        let mut hub = hub_of(PAGE_SIZE * 3);
        hub.turn_page(2);
        assert_eq!(hub.page_index, 2);

        // The search narrows underneath the reader without a reset — the case a
        // catalogue arriving late produces, where `sync_search` never ran.
        hub.terms = vec!["a1".into()];
        let page = hub.page();
        assert!(!page.is_empty(), "a narrowed list must not show an empty grid");
        assert_eq!(page.len(), hub.visible().len().min(PAGE_SIZE));
    }

    #[test]
    fn search_narrows_the_visible_set() {
        let mut hub = Hub::default();
        hub.kind = Kind::Models;
        hub.catalogues[Kind::Models.index()] = Catalogue::Ready(vec![
            Asset {
                slug: "arm_chair".into(),
                name: "Arm Chair".into(),
                ..Default::default()
            },
            Asset {
                slug: "old_barrel".into(),
                name: "Old Barrel".into(),
                ..Default::default()
            },
        ]);
        assert_eq!(hub.visible().len(), 2);
        hub.terms = vec!["chair".into()];
        assert_eq!(hub.visible().len(), 1);
    }
}
