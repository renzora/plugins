//! Panel state, the worker thread each generation runs on, and where its model
//! lands in the project.
//!
//! # Threads and the inbox
//!
//! A generation is minutes of blocking HTTP (see [`crate::http`]), so each one
//! gets its own thread and reports back through [`Inbox`], which
//! [`crate::drain_inbox`] empties once a frame. `Arc<Mutex<Vec<_>>>` rather than
//! an `mpsc` pair because a Bevy resource must be `Sync` and a `Receiver` is
//! not; it is the same arrangement the Poly Haven plugin and
//! `renzora_scripting`'s `HttpInbox` use.

use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};

use bevy::prelude::*;

use crate::providers::{Provider, Report, Spec, CANCELLED};

/// What a job is generated from, as the panel's tab selects it.
#[derive(Clone, Copy, PartialEq, Eq, Debug, Default)]
pub enum Mode {
    #[default]
    Text,
    Image,
}

impl Mode {
    pub fn index(self) -> usize {
        match self {
            Mode::Text => 0,
            Mode::Image => 1,
        }
    }

    pub fn from_index(index: usize) -> Self {
        if index == 1 { Mode::Image } else { Mode::Text }
    }
}

/// What a worker thread sends back.
pub enum Msg {
    Progress {
        id: u64,
        percent: Option<u8>,
        stage: String,
    },
    Thumbnail {
        id: u64,
        url: String,
    },
    /// `Ok` carries the project-relative path of the written model.
    Finished {
        id: u64,
        result: Result<String, String>,
    },
}

#[derive(Clone, Default)]
pub struct Inbox(Arc<Mutex<Vec<Msg>>>);

impl Inbox {
    fn push(&self, msg: Msg) {
        // Poisoned means a worker panicked holding the lock. Dropping its message
        // beats propagating that panic into the frame loop.
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

/// One generation, running or finished.
pub struct Job {
    pub id: u64,
    /// The prompt, or the image's file name. What the row is labelled with.
    pub title: String,
    pub provider: Provider,
    /// Absolute folder the model is written into. Kept because a finished job
    /// is handed to the import pipeline by folder.
    pub folder: PathBuf,
    pub percent: Option<u8>,
    pub stage: String,
    pub thumbnail: Option<String>,
    /// `None` while running.
    pub outcome: Option<Result<String, String>>,
    /// Set once the folder is queued for import, so that happens exactly once.
    pub handed_off: bool,
    /// Shared with the worker thread, which checks it between requests. A flag
    /// rather than a message because the worker spends most of its life asleep
    /// in a poll loop, and a flag is something it can glance at while waiting.
    pub cancel: Arc<AtomicBool>,
}

impl Job {
    pub fn running(&self) -> bool {
        self.outcome.is_none()
    }

    /// Cancel pressed, and the worker has not yet noticed.
    pub fn cancelling(&self) -> bool {
        self.running() && self.cancel.load(Ordering::Relaxed)
    }

    /// Ended because the user cancelled it, as opposed to failing.
    pub fn was_cancelled(&self) -> bool {
        matches!(&self.outcome, Some(Err(why)) if why == CANCELLED)
    }
}

#[derive(Resource, Default)]
pub struct Hub {
    pub mode: Mode,
    pub prompt: String,
    pub image_path: String,
    /// Newest first.
    pub jobs: Vec<Job>,
    pub inbox: Inbox,
    next_id: u64,
}

/// How many finished jobs stay listed. Running ones are never trimmed.
const JOB_HISTORY: usize = 8;

impl Hub {
    pub fn job(&self, id: u64) -> Option<&Job> {
        self.jobs.iter().find(|j| j.id == id)
    }

    pub fn running(&self) -> usize {
        self.jobs.iter().filter(|j| j.running()).count()
    }

    /// Record a new job and return its id.
    pub fn push(&mut self, title: String, provider: Provider, folder: PathBuf) -> u64 {
        self.next_id += 1;
        let id = self.next_id;
        self.jobs.insert(
            0,
            Job {
                id,
                title,
                provider,
                folder,
                percent: Some(0),
                stage: "submitting".to_string(),
                thumbnail: None,
                outcome: None,
                handed_off: false,
                cancel: Arc::new(AtomicBool::new(false)),
            },
        );
        id
    }

    /// Ask a running job to stop. The worker ends it with [`CANCELLED`].
    pub fn cancel(&mut self, id: u64) {
        if let Some(job) = self.jobs.iter_mut().find(|j| j.id == id && j.running()) {
            job.cancel.store(true, Ordering::Relaxed);
            job.stage = "cancelling".to_string();
        }
    }

    pub fn apply(&mut self, msg: Msg) {
        match msg {
            Msg::Progress { id, percent, stage } => {
                // A cancelled job keeps saying "cancelling" until it ends, rather
                // than flicking back to a stage a poll reported just before.
                if let Some(job) = self.jobs.iter_mut().find(|j| j.id == id && !j.cancelling()) {
                    job.percent = percent;
                    job.stage = stage;
                }
            }
            Msg::Thumbnail { id, url } => {
                if let Some(job) = self.jobs.iter_mut().find(|j| j.id == id) {
                    job.thumbnail = Some(url);
                }
            }
            Msg::Finished { id, result } => {
                if let Some(job) = self.jobs.iter_mut().find(|j| j.id == id) {
                    match &result {
                        Ok(path) => info!("[ai_mesh] {} -> {path}", job.title),
                        Err(why) if why == CANCELLED => info!("[ai_mesh] {} cancelled", job.title),
                        Err(why) => warn!("[ai_mesh] {} failed: {why}", job.title),
                    }
                    job.outcome = Some(result);
                }
            }
        }
        while self.jobs.len() > JOB_HISTORY {
            let Some(oldest) = self.jobs.iter().rposition(|j| !j.running()) else {
                break;
            };
            self.jobs.remove(oldest);
        }
    }

    /// Is `folder` already the target of a listed job? Two jobs with the same
    /// prompt queued in one frame would otherwise both see it as free.
    fn claims(&self, folder: &Path) -> bool {
        self.jobs.iter().any(|j| j.folder == folder)
    }
}

/// Start `spec` on its own thread. The model is written to `folder/<stem>.glb`.
#[allow(clippy::too_many_arguments)]
pub fn spawn(
    provider: Provider,
    spec: Spec,
    id: u64,
    cancel: Arc<AtomicBool>,
    folder: PathBuf,
    stem: String,
    label: String,
    inbox: Inbox,
) {
    std::thread::spawn(move || {
        let progress_inbox = inbox.clone();
        let progress = move |percent: Option<u8>, stage: &str| {
            progress_inbox.push(Msg::Progress {
                id,
                percent,
                stage: stage.to_string(),
            });
        };
        let thumb_inbox = inbox.clone();
        let thumbnail = move |url: String| thumb_inbox.push(Msg::Thumbnail { id, url });
        let report = Report {
            progress: &progress,
            thumbnail: &thumbnail,
            cancel: &cancel,
        };

        let result = provider.run(&spec, &report).and_then(|output| {
            // Checked again here: a cancel pressed during the download must still
            // mean nothing lands in the project and nothing is imported.
            report.check()?;
            if !output.glb.starts_with(b"glTF") {
                // The magic number, checked before anything is written: a signed
                // URL that expired returns an XML error page with a 200 on some
                // storage backends, and the importer would report that as a
                // corrupt model the user then goes looking for.
                return Err("the downloaded file is not a GLB model".to_string());
            }
            std::fs::create_dir_all(&folder).map_err(|e| format!("{}: {e}", folder.display()))?;
            let path = folder.join(format!("{stem}.glb"));
            std::fs::write(&path, &output.glb).map_err(|e| format!("{}: {e}", path.display()))?;
            Ok(format!("{label}/{stem}.glb"))
        });
        inbox.push(Msg::Finished { id, result });
    });
}

/// The folder a new model goes into: `generated/<slug>` under the project's
/// `models/` when it has one, else under the project root.
///
/// One folder per model because the import pipeline extracts a `textures/` and
/// a `.material` per material beside the GLB, and two models sharing a folder
/// would interleave files that must move together. A taken name gets a numeric
/// suffix rather than overwriting a model the scene may already use.
pub fn destination(project_root: &Path, hub: &Hub, title: &str) -> (PathBuf, String) {
    let models = project_root.join("models");
    let base = if models.is_dir() { models } else { project_root.to_path_buf() };
    let base = base.join("generated");
    let stem = slug(title);
    let mut folder = base.join(&stem);
    let mut n = 2;
    while folder.exists() || hub.claims(&folder) {
        folder = base.join(format!("{stem}_{n}"));
        n += 1;
    }
    (folder, stem)
}

/// A file-system-safe name from a prompt: lowercase words joined with `_`, cut
/// at a word boundary so a long prompt does not become a long path. Windows
/// still caps most paths at 260 characters, and a project is often deep.
pub fn slug(title: &str) -> String {
    let mut out = String::new();
    for word in title
        .split(|c: char| !c.is_ascii_alphanumeric())
        .filter(|w| !w.is_empty())
    {
        if out.len() + word.len() > 32 {
            break;
        }
        if !out.is_empty() {
            out.push('_');
        }
        out.push_str(&word.to_ascii_lowercase());
    }
    if out.is_empty() { "model".to_string() } else { out }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn slugs_are_short_and_safe() {
        assert_eq!(slug("A rusty iron treasure-chest!"), "a_rusty_iron_treasure_chest");
        assert_eq!(slug("???"), "model");
        assert!(slug(&"word ".repeat(50)).len() <= 32);
    }

    #[test]
    fn a_claimed_folder_is_not_reused() {
        let root = std::env::temp_dir().join("ai_mesh_test_no_such_project");
        let mut hub = Hub::default();
        let (first, _) = destination(&root, &hub, "crate");
        hub.push("crate".into(), Provider::Meshy, first.clone());
        let (second, _) = destination(&root, &hub, "crate");
        assert_ne!(first, second);
    }

    #[test]
    fn cancel_flags_only_a_running_job_and_ends_neutral() {
        let mut hub = Hub::default();
        let id = hub.push("crate".into(), Provider::Meshy, PathBuf::from("a"));
        hub.cancel(id);
        assert!(hub.job(id).unwrap().cancelling());
        // A late progress report must not overwrite "cancelling".
        hub.apply(Msg::Progress { id, percent: Some(40), stage: "generating".into() });
        assert_eq!(hub.job(id).unwrap().stage, "cancelling");
        hub.apply(Msg::Finished { id, result: Err(CANCELLED.into()) });
        let job = hub.job(id).unwrap();
        assert!(job.was_cancelled() && !job.running() && !job.cancelling());

        let done = hub.push("barrel".into(), Provider::Meshy, PathBuf::from("b"));
        hub.apply(Msg::Finished { id: done, result: Ok(String::new()) });
        hub.cancel(done);
        assert!(!hub.job(done).unwrap().cancel.load(Ordering::Relaxed), "a finished job is left alone");
    }

    #[test]
    fn finished_jobs_are_trimmed_and_running_ones_are_not() {
        let mut hub = Hub::default();
        for i in 0..JOB_HISTORY + 3 {
            let id = hub.push(format!("j{i}"), Provider::Meshy, PathBuf::from(format!("f{i}")));
            if i > 0 {
                hub.apply(Msg::Finished { id, result: Ok(String::new()) });
            }
        }
        assert_eq!(hub.jobs.len(), JOB_HISTORY);
        assert!(hub.jobs.iter().any(|j| j.title == "j0"), "the running job survives");
    }
}
