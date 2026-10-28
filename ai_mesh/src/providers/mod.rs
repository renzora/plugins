//! The services a model can come from, and the one shape they are all driven
//! through.
//!
//! Every hosted 3D API is the same three steps under different names: submit a
//! job, poll until it settles, download a GLB. So a provider here is a single
//! function, [`Provider::run`], that does all three on a worker thread and
//! returns the model's bytes. Everything around it (the panel, the queue, the
//! import) is shared, which is what makes adding a fifth service one file.

mod fal;
mod local;
mod meshy;
mod tripo;

use std::sync::atomic::{AtomicBool, Ordering};
use std::time::{Duration, Instant};

/// Which service a job runs on.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Provider {
    Meshy,
    Tripo,
    /// Tencent's Hunyuan3D v3, hosted by fal.ai.
    FalHunyuan,
    /// Microsoft's TRELLIS 2, hosted by fal.ai.
    FalTrellis,
    /// Hunyuan3D 2.1's own `api_server.py`, on the user's GPU.
    LocalHunyuan,
}

/// Where a provider's credential is kept. Both fal models share one, because
/// it is one fal account; asking for the same key twice would be a chore that
/// also lets the two drift apart.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum KeySlot {
    Meshy,
    Tripo,
    Fal,
}

impl Provider {
    /// In dropdown order, which is also the order [`Self::index`] numbers.
    pub const ALL: [Provider; 5] = [
        Provider::Meshy,
        Provider::Tripo,
        Provider::FalHunyuan,
        Provider::FalTrellis,
        Provider::LocalHunyuan,
    ];

    pub fn index(self) -> usize {
        Self::ALL.iter().position(|p| *p == self).unwrap_or(0)
    }

    pub fn from_index(index: usize) -> Self {
        Self::ALL.get(index).copied().unwrap_or(Provider::Meshy)
    }

    pub fn label(self) -> &'static str {
        match self {
            Provider::Meshy => "Meshy",
            Provider::Tripo => "Tripo",
            Provider::FalHunyuan => "Hunyuan3D (fal.ai)",
            Provider::FalTrellis => "TRELLIS 2 (fal.ai)",
            Provider::LocalHunyuan => "Hunyuan3D (local server)",
        }
    }

    /// Short id written to the settings file. Stable across releases, unlike
    /// the dropdown index, so reordering the list never changes anyone's pick.
    pub fn id(self) -> &'static str {
        match self {
            Provider::Meshy => "meshy",
            Provider::Tripo => "tripo",
            Provider::FalHunyuan => "fal-hunyuan3d",
            Provider::FalTrellis => "fal-trellis2",
            Provider::LocalHunyuan => "local-hunyuan3d",
        }
    }

    pub fn from_id(id: &str) -> Option<Self> {
        Self::ALL.into_iter().find(|p| p.id() == id)
    }

    /// TRELLIS and the local Hunyuan3D server are image-to-3D models only.
    pub fn takes_text(self) -> bool {
        !matches!(self, Provider::FalTrellis | Provider::LocalHunyuan)
    }

    pub fn takes_image(self) -> bool {
        true
    }

    /// `None` for the local server, which has no auth at all.
    pub fn key_slot(self) -> Option<KeySlot> {
        match self {
            Provider::Meshy => Some(KeySlot::Meshy),
            Provider::Tripo => Some(KeySlot::Tripo),
            Provider::FalHunyuan | Provider::FalTrellis => Some(KeySlot::Fal),
            Provider::LocalHunyuan => None,
        }
    }

    /// Submit, wait, download. Blocking: runs on the job's own thread.
    pub fn run(self, spec: &Spec, report: &Report) -> Result<Output, String> {
        // Checked before anything is submitted, so a job cancelled while its
        // thread was still starting spends nothing at all.
        report.check()?;
        match self {
            Provider::Meshy => meshy::run(spec, report),
            Provider::Tripo => tripo::run(spec, report),
            Provider::FalHunyuan => fal::run(fal::Model::Hunyuan, spec, report),
            Provider::FalTrellis => fal::run(fal::Model::Trellis, spec, report),
            Provider::LocalHunyuan => local::run(spec, report),
        }
    }
}

/// What a model is generated from.
pub enum Input {
    Text(String),
    /// Read from disk on the main thread when the job was queued, so a file the
    /// user moves afterwards cannot change what is sent.
    Image {
        bytes: Vec<u8>,
        mime: &'static str,
        filename: String,
    },
}

/// Everything a provider needs, owned, so the worker thread never reaches back
/// into the World.
pub struct Spec {
    pub input: Input,
    /// Ask for metallic, roughness and normal maps, not only a base colour.
    pub pbr: bool,
    /// Ask for a game-budget mesh rather than the provider's (often
    /// million-triangle) default.
    pub low_poly: bool,
    /// The credential for this provider's [`KeySlot`]; empty for the local one.
    pub key: String,
    /// Base URL of the local server. Unused by the hosted providers.
    pub local_url: String,
}

/// A finished generation.
pub struct Output {
    pub glb: Vec<u8>,
}

/// Progress callbacks from a worker, posted back to the panel.
pub struct Report<'a> {
    /// `(percent, stage)`. `None` percent means the provider gives no number
    /// (fal's queue only says "in progress"), so the panel shows a busy bar.
    pub progress: &'a dyn Fn(Option<u8>, &str),
    /// A preview image URL, as soon as the provider offers one.
    pub thumbnail: &'a dyn Fn(String),
    /// Set by the panel's cancel button. Read, never written, from here.
    pub cancel: &'a AtomicBool,
}

/// The error a cancelled job ends with. Compared by the panel, which shows it
/// as a neutral outcome rather than a red failure: the user asked for it.
pub const CANCELLED: &str = "cancelled";

impl Report<'_> {
    pub fn cancelled(&self) -> bool {
        self.cancel.load(Ordering::Relaxed)
    }

    /// `Err(CANCELLED)` once the user has cancelled. For the points between
    /// requests where a provider would otherwise start the next paid step.
    pub fn check(&self) -> Result<(), String> {
        if self.cancelled() {
            Err(CANCELLED.to_string())
        } else {
            Ok(())
        }
    }
}

/// The triangle budget `low_poly` asks for. A prop-sized game asset; the same
/// number across providers so switching service does not change what "low
/// poly" means.
pub const LOW_POLY_FACES: u32 = 10_000;

/// How long a job may run before it is called lost.
///
/// Hosted generation is 30 s to 3 min; a refine pass or a busy queue can double
/// that. The local server needs a hard stop most of all: when its worker
/// throws, `/status` keeps answering "processing" forever.
const JOB_TIMEOUT: Duration = Duration::from_secs(15 * 60);

/// How often a waiting job looks at its cancel flag. Short, so Cancel feels
/// immediate, and independent of the poll rate, which is the provider's to set.
const CANCEL_CHECK: Duration = Duration::from_millis(250);

/// Poll `check` every `every` until it returns a value, an error, the user
/// cancels, or the job runs out of time.
///
/// Every provider documents a poll rate (Tripo: no more than once a second,
/// two recommended), and a polling loop is where an editor plugin can quietly
/// spend a user's rate limit, so the interval is passed in rather than shared.
///
/// `on_cancel` asks the provider to stop the remote task, where it has a way
/// to. It is best-effort and its result is ignored: the job is over on this
/// side either way, and a failed cancel request must not turn a cancellation
/// into an error message.
fn poll<T>(
    every: Duration,
    report: &Report,
    on_cancel: impl FnOnce(),
    mut check: impl FnMut() -> Result<Option<T>, String>,
) -> Result<T, String> {
    let started = Instant::now();
    loop {
        if report.cancelled() {
            on_cancel();
            return Err(CANCELLED.to_string());
        }
        if let Some(done) = check()? {
            return Ok(done);
        }
        if started.elapsed() > JOB_TIMEOUT {
            return Err(format!(
                "gave up after {} minutes without a result",
                JOB_TIMEOUT.as_secs() / 60
            ));
        }
        // Slept in slices so a cancel lands within a quarter second rather
        // than after the provider's full poll interval.
        let wake = Instant::now() + every;
        while Instant::now() < wake && !report.cancelled() {
            std::thread::sleep(CANCEL_CHECK.min(wake.saturating_duration_since(Instant::now())));
        }
    }
}

/// A percentage field, clamped, from any JSON number.
fn percent(value: Option<&renzora::serde_json::Value>) -> Option<u8> {
    value
        .and_then(|v| v.as_f64())
        .map(|p| p.clamp(0.0, 100.0) as u8)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn ids_round_trip_and_are_unique() {
        for provider in Provider::ALL {
            assert_eq!(Provider::from_id(provider.id()), Some(provider));
            assert_eq!(Provider::from_index(provider.index()), provider);
        }
    }

    #[test]
    fn image_only_models_refuse_text() {
        assert!(!Provider::FalTrellis.takes_text());
        assert!(!Provider::LocalHunyuan.takes_text());
        assert!(Provider::Meshy.takes_text());
    }
}
