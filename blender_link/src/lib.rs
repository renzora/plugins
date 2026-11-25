//! Live link from Blender.
//!
//! Work in Blender and the editor follows: objects, their meshes with every
//! modifier applied, materials (Principled BSDF factors, base colour and
//! normal textures) and lights arrive as you change them, and are saved into
//! the project as one `.rvg` model per object, so the scene works with Blender
//! closed, in Play, and in an exported game.
//!
//! Blender runs the add-on in `addon/renzora_live_link.py`, which this plugin
//! installs and enables in every Blender 4.2+ it finds (see [`install`]) and
//! which connects back on `127.0.0.1:47810` (`RENZORA_BLENDER_PORT` to change it). The
//! link is one way, Blender to Renzora: Blender is the source of truth for the
//! objects it links, so an edit made to one here is overwritten by the next
//! change in Blender.
//!
//! | module | what it is |
//! |---|---|
//! | [`server`] | the listener and its threads, and the frame format |
//! | [`protocol`] | each message decoded from its header and payload |
//! | [`apply`] | messages turned into entities, and the baking into `.rvg` |
//! | [`glb`] | the binary glTF writer |
//! | [`install`] | installs the add-on into Blender and enables it |

use bevy::prelude::*;
use renzora::core::console_log::{console_error, console_info};
use renzora::{RenzoraShellExt, ShellStatusAlign, ShellStatusItem, ShellStatusSegment};
use std::sync::mpsc::Receiver;
use std::sync::{Arc, Mutex};

mod apply;
mod glb;
mod install;
mod protocol;
mod server;

pub use apply::{BlenderLink, BlenderLinkRoot};

/// The console category every line from this plugin carries.
pub const CATEGORY: &str = "blender";

/// The port the add-on connects to unless `RENZORA_BLENDER_PORT` says
/// otherwise. Fixed, as the MCP plugin's is, because it is typed into the
/// add-on once and a port that moved would silently point it at nothing.
pub const DEFAULT_PORT: u16 = 47810;

/// The main-thread end of the socket threads' queue.
#[derive(Resource)]
pub struct LinkBridge {
    pub rx: Mutex<Receiver<server::Incoming>>,
    pub state: Arc<server::ServerState>,
}

pub struct BlenderLinkPlugin;

impl Plugin for BlenderLinkPlugin {
    fn build(&self, app: &mut App) {
        let port = std::env::var("RENZORA_BLENDER_PORT")
            .ok()
            .and_then(|p| p.parse::<u16>().ok())
            .unwrap_or(DEFAULT_PORT);
        let state = Arc::new(server::ServerState::new(port));
        let (tx, rx) = std::sync::mpsc::channel();
        server::spawn(Arc::clone(&state), tx);

        app.register_type::<BlenderLink>()
            .register_type::<BlenderLinkRoot>()
            .insert_resource(LinkBridge { rx: Mutex::new(rx), state })
            .init_resource::<apply::Session>()
            .add_systems(Update, (apply::pump, apply::drop_loaded_models).chain())
            .add_systems(Startup, announce);

        app.register_shell_status_item(ShellStatusItem {
            id: "blender_link",
            align: ShellStatusAlign::Right,
            order: -2,
            render: status_segments,
        });
    }
}

fn announce(bridge: Res<LinkBridge>) {
    let port = bridge.state.port;
    if bridge.state.phase() == server::Phase::Failed {
        console_error(
            CATEGORY,
            format!("Blender link could not listen on 127.0.0.1:{port}: {}", bridge.state.detail()),
        );
        return;
    }
    console_info(CATEGORY, format!("Blender link listening on 127.0.0.1:{port}"));

    // Off the main thread: enabling the add-on starts a headless Blender,
    // which takes seconds.
    if let Err(e) = std::thread::Builder::new()
        .name("blender-link-install".into())
        .spawn(install::run)
    {
        console_error(CATEGORY, format!("could not start the Blender add-on install ({e})"));
    }
}

/// One status-bar segment: idle, linked (with the file and object count), or
/// the bind failure.
fn status_segments(world: &World) -> Vec<ShellStatusSegment> {
    let Some(bridge) = world.get_resource::<LinkBridge>() else {
        return Vec::new();
    };
    const IDLE: [u8; 3] = [150, 150, 164];
    const LIVE: [u8; 3] = [232, 125, 13];
    const BAD: [u8; 3] = [220, 80, 80];

    let session = world.get_resource::<apply::Session>();
    let segment = match bridge.state.phase() {
        server::Phase::Failed => ShellStatusSegment::new(
            "link-break",
            format!("Blender {}", bridge.state.detail()),
            BAD,
        ),
        _ => match session.filter(|s| s.connected() && !s.file.is_empty()) {
            Some(s) => ShellStatusSegment::new(
                "link",
                format!("Blender {} ({})", s.file, s.object_count()),
                LIVE,
            ),
            None => ShellStatusSegment::new("link", "Blender", IDLE),
        },
    };
    vec![segment]
}

// Editor scope: the link edits the open scene and writes into the project,
// neither of which a shipped game has, and a game has no business opening a
// port that accepts geometry. The baked model files are what reach the game.
renzora::plugin!(BlenderLinkPlugin);
