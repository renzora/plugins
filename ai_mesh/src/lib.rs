//! AI Mesh Generator: describe a model, or hand over a reference image, and get
//! a textured GLB imported into the open project.
//!
//! One panel (`ai_mesh`, under **Assets** in the Add-Panel picker) and one
//! settings section (**Settings > Plugins > AI Mesh**) for the API keys. Each
//! generation runs on its own thread against one of five backends (Meshy,
//! Tripo, Hunyuan3D or TRELLIS 2 on fal.ai, or a local Hunyuan3D server), and
//! the finished model is written to `generated/<name>/` and handed to the
//! engine's import pipeline.
//!
//! # Bring your own account
//!
//! There is no Renzora service in the middle. The user pastes their own key for
//! the service they use; requests go straight from the editor to that service
//! and spend that account's credits. That keeps billing, rate limits and each
//! service's output licence between the user and the service, and it means the
//! plugin works the day a new account is opened.
//!
//! # What it deliberately does not do
//!
//! No conversion, no material authoring, no scene placement. A GLB lands in the
//! project and `ImportInPlaceQueue` gives it the same treatment a model dropped
//! into the viewport gets: textures extracted, a `.material` per material. A
//! generator that built materials itself would be deciding things that belong
//! to the importer, and would go stale when the importer changed.

mod http;
mod job;
mod panel;
mod providers;
mod settings;

use bevy::prelude::*;

use renzora::RenzoraShellExt;
use renzora_ember::panel::RegisterPanelContent;
use renzora_ember::settings_sections::RegisterSettingsSection;
use renzora_ember::UiInteraction;

use job::{Hub, Mode};
use providers::{Input, Spec};
use settings::Settings;

/// The largest reference image any of the providers accepts.
const MAX_IMAGE_BYTES: u64 = 20 * 1024 * 1024;

#[derive(Default)]
pub struct AiMeshPlugin;

impl Plugin for AiMeshPlugin {
    fn build(&self, app: &mut App) {
        info!("[editor] AiMeshPlugin");
        app.insert_resource(settings::load());
        app.init_resource::<Hub>();
        // `renzora_import_ui` inits this too. `init_resource` is insert-if-absent,
        // so doing it here removes the assumption that the importer built first
        // without being able to clobber its queue.
        app.init_resource::<renzora::core::ImportInPlaceQueue>();

        app.register_shell_panel("ai_mesh", "AI Mesh", "magic-wand", "Assets");
        let mut panel = app.register_panel_content("ai_mesh", true, panel::build);
        panel
            .app()
            .register_settings_section("ai_mesh", "AI Mesh", "magic-wand", settings::build);
        panel
            .systems(
                Update,
                (on_generate, on_choose_image, on_cancel_job, panel::apply_thumbnails),
            )
            // A generation takes minutes and the user will be in another tab for
            // most of them. Gating these on the panel would stall progress and,
            // worse, hold a finished model back from the importer until the tab
            // was looked at. Settings are edited in the Settings panel, so saving
            // them cannot wait on this one either.
            .always(Update, (drain_inbox, settings::persist));
    }
}

/// Start a generation.
fn on_generate(
    buttons: Query<&UiInteraction, (With<panel::GenerateButton>, Changed<UiInteraction>)>,
    settings: Res<Settings>,
    mut hub: ResMut<Hub>,
    project: Option<Res<renzora::CurrentProject>>,
) {
    if !buttons.iter().any(|i| *i == UiInteraction::Pressed) {
        return;
    }
    if let Some(why) = panel::blocker(&settings, &hub, project.is_some()) {
        info!("[ai_mesh] not generating: {why}");
        return;
    }
    let Some(project) = project else { return };

    let provider = settings.provider();
    let (input, title) = match hub.mode {
        Mode::Text => {
            let prompt = hub.prompt.trim().to_string();
            (Input::Text(prompt.clone()), prompt)
        }
        Mode::Image => match read_image(hub.image_path.trim()) {
            Ok((input, title)) => (input, title),
            Err(why) => {
                // Reported as a failed job, not only logged, because the panel's
                // job list is where the user is looking after pressing Generate.
                let title = hub.image_path.clone();
                let id = hub.push(title, provider, Default::default());
                hub.apply(job::Msg::Finished { id, result: Err(why) });
                return;
            }
        },
    };

    let (folder, stem) = job::destination(&project.path, &hub, &title);
    let label = project
        .make_relative(&folder)
        .unwrap_or_else(|| folder.display().to_string());
    let spec = Spec {
        input,
        pbr: settings.pbr,
        low_poly: settings.low_poly,
        key: provider
            .key_slot()
            .map(|slot| settings.key(slot).to_string())
            .unwrap_or_default(),
        local_url: settings.local_url.clone(),
    };

    let id = hub.push(title, provider, folder.clone());
    let Some(cancel) = hub.job(id).map(|j| j.cancel.clone()) else { return };
    let inbox = hub.inbox.clone();
    job::spawn(provider, spec, id, cancel, folder, stem, label, inbox);
}

/// Stop a running generation.
fn on_cancel_job(
    buttons: Query<(&UiInteraction, &panel::CancelJob), Changed<UiInteraction>>,
    mut hub: ResMut<Hub>,
) {
    for (interaction, button) in &buttons {
        if *interaction == UiInteraction::Pressed {
            hub.cancel(button.0);
        }
    }
}

/// Read and check a reference image on the main thread, before any credits are
/// spent on it.
fn read_image(path: &str) -> Result<(Input, String), String> {
    let path = std::path::Path::new(path);
    let mime = http::image_mime(path).ok_or("the image must be a PNG, JPEG or WebP file")?;
    let size = std::fs::metadata(path)
        .map_err(|e| format!("{}: {e}", path.display()))?
        .len();
    if size > MAX_IMAGE_BYTES {
        return Err("the image is over 20 MB, the most any service accepts".to_string());
    }
    let bytes = std::fs::read(path).map_err(|e| format!("{}: {e}", path.display()))?;
    let filename = path
        .file_name()
        .map(|n| n.to_string_lossy().into_owned())
        .unwrap_or_else(|| "image".to_string());
    let title = path
        .file_stem()
        .map(|n| n.to_string_lossy().into_owned())
        .unwrap_or_else(|| "image".to_string());
    Ok((
        Input::Image {
            bytes,
            mime,
            filename,
        },
        title,
    ))
}

/// Open the native file dialog for a reference image.
fn on_choose_image(
    buttons: Query<&UiInteraction, (With<panel::ChooseImageButton>, Changed<UiInteraction>)>,
    mut commands: Commands,
) {
    if !buttons.iter().any(|i| *i == UiInteraction::Pressed) {
        return;
    }
    commands.queue(|world: &mut World| {
        let Some(file) = rfd::FileDialog::new()
            .set_title("Choose a reference image")
            .add_filter("Image", &["png", "jpg", "jpeg", "webp"])
            .pick_file()
        else {
            return;
        };
        if let Some(mut hub) = world.get_resource_mut::<Hub>() {
            hub.image_path = file.display().to_string();
        }
    });
}

/// Fold worker messages into the hub, and hand each finished model to the
/// import pipeline.
fn drain_inbox(mut hub: ResMut<Hub>, mut imports: ResMut<renzora::core::ImportInPlaceQueue>) {
    let messages = hub.inbox.drain();
    if messages.is_empty() {
        // Return before touching `hub` mutably: an unconditional deref marks it
        // changed every frame and re-runs every binding in the panel.
        return;
    }
    for message in messages {
        hub.apply(message);
    }
    // Without this a generated GLB loads untextured: no extracted `textures/`
    // and no `.material` per material. The importer lives in a crate a native
    // plugin cannot link, so the request crosses as a folder path through the
    // contract crate.
    for job in hub.jobs.iter_mut() {
        if !job.handed_off && matches!(job.outcome, Some(Ok(_))) {
            imports.0.push(job.folder.clone());
            job.handed_off = true;
        }
    }
}

// `plugin!`, not `add!`: a native plugin, compiled against the SDK and loaded
// from `plugins/`. `Editor` is the default and is what this wants: a game has
// no panel to generate from and no project to import into, and a shipped game
// must never carry anyone's API key.
renzora::plugin!(AiMeshPlugin);
