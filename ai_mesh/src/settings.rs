//! Credentials and preferences, and the **Settings > Plugins > AI Mesh**
//! section that edits them.
//!
//! # Where the keys live
//!
//! In the user's own config directory (`%APPDATA%\renzora\ai_mesh.json`,
//! `~/.config/renzora/ai_mesh.json` elsewhere), never in the project. A project
//! is shared, committed and exported; a key written into it would ship inside
//! every build of the game and every repository it was pushed to. The file is
//! plain JSON like the AI Chat plugin's, which is the norm for local developer
//! tooling, and it is read by nothing but this plugin.
//!
//! Every provider is bring-your-own-account: the key is the user's, the credits
//! are theirs, and requests go from their machine straight to the provider.

use bevy::prelude::*;

use renzora::serde::{Deserialize, Serialize};
use renzora_ember::font::{ui_font, EmberFonts};
use renzora_ember::theme::{rgb, text_muted};
use renzora_ember::widgets::{bind_text_input, password_input, text_input};

use crate::providers::{KeySlot, Provider};

/// The address `api_server.py` listens on when started with no arguments.
pub const DEFAULT_LOCAL_URL: &str = "http://127.0.0.1:8081";

#[derive(Resource, Clone, PartialEq, Serialize, Deserialize)]
#[serde(crate = "renzora::serde", default)]
pub struct Settings {
    /// [`Provider::id`], not an index, so the saved pick survives the list
    /// being reordered.
    pub provider: String,
    pub meshy_key: String,
    pub tripo_key: String,
    pub fal_key: String,
    pub local_url: String,
    pub pbr: bool,
    pub low_poly: bool,
}

impl Default for Settings {
    fn default() -> Self {
        Self {
            provider: Provider::Meshy.id().to_string(),
            meshy_key: String::new(),
            tripo_key: String::new(),
            fal_key: String::new(),
            local_url: DEFAULT_LOCAL_URL.to_string(),
            pbr: true,
            // On by default: this is a game engine, and every provider's own
            // default is a mesh in the hundreds of thousands of triangles.
            low_poly: true,
        }
    }
}

impl Settings {
    pub fn provider(&self) -> Provider {
        Provider::from_id(&self.provider).unwrap_or(Provider::Meshy)
    }

    pub fn key(&self, slot: KeySlot) -> &str {
        match slot {
            KeySlot::Meshy => &self.meshy_key,
            KeySlot::Tripo => &self.tripo_key,
            KeySlot::Fal => &self.fal_key,
        }
    }

    fn key_mut(&mut self, slot: KeySlot) -> &mut String {
        match slot {
            KeySlot::Meshy => &mut self.meshy_key,
            KeySlot::Tripo => &mut self.tripo_key,
            KeySlot::Fal => &mut self.fal_key,
        }
    }
}

fn config_path() -> Option<std::path::PathBuf> {
    let base = if cfg!(windows) {
        std::env::var_os("APPDATA").map(std::path::PathBuf::from)?
    } else {
        std::path::PathBuf::from(std::env::var_os("HOME")?).join(".config")
    };
    Some(base.join("renzora").join("ai_mesh.json"))
}

pub fn load() -> Settings {
    config_path()
        .and_then(|path| std::fs::read_to_string(path).ok())
        .and_then(|text| renzora::serde_json::from_str(&text).ok())
        .unwrap_or_default()
}

fn save(settings: &Settings) {
    let Some(path) = config_path() else { return };
    if let Some(dir) = path.parent() {
        let _ = std::fs::create_dir_all(dir);
    }
    if let Ok(json) = renzora::serde_json::to_string_pretty(settings) {
        let _ = std::fs::write(path, json);
    }
}

/// Write the settings out when they change, at most once a second.
///
/// Debounced because a key is typed one character at a time and each one is a
/// change; compared against the last write because `ResMut` marks the resource
/// changed on any mutable access, even one that wrote the same value.
pub fn persist(
    settings: Res<Settings>,
    time: Res<Time>,
    mut pending: Local<bool>,
    mut last_saved: Local<Option<Settings>>,
    mut last_save_at: Local<f64>,
) {
    if settings.is_changed() {
        *pending = true;
    }
    let now = time.elapsed_secs_f64();
    if !*pending || now - *last_save_at < 1.0 {
        return;
    }
    if last_saved.as_ref() != Some(&*settings) {
        save(&settings);
        *last_saved = Some(settings.clone());
    }
    *pending = false;
    *last_save_at = now;
}

/// Settings > Plugins > AI Mesh.
pub fn build(commands: &mut Commands, fonts: &EmberFonts) -> Entity {
    let col = commands
        .spawn(Node {
            width: Val::Percent(100.0),
            flex_direction: FlexDirection::Column,
            row_gap: Val::Px(6.0),
            ..default()
        })
        .id();

    let meshy = key_row(commands, fonts, "Meshy key", KeySlot::Meshy);
    let tripo = key_row(commands, fonts, "Tripo key", KeySlot::Tripo);
    let fal = key_row(commands, fonts, "fal.ai key", KeySlot::Fal);

    let local = row(commands, fonts, "Local server");
    let url = text_input(commands, &fonts.ui, DEFAULT_LOCAL_URL, "");
    grow(commands, url);
    bind_text_input(
        commands,
        url,
        |rx| {
            rx.get_resource::<Settings>()
                .map(|s| s.local_url.clone())
                .unwrap_or_default()
        },
        |world, value| {
            if let Some(mut s) = world.get_resource_mut::<Settings>() {
                let value = value.trim().trim_end_matches('/');
                let value = if value.is_empty() { DEFAULT_LOCAL_URL } else { value };
                if s.local_url != value {
                    s.local_url = value.to_string();
                }
            }
        },
    );
    commands.entity(local).add_child(url);

    let note = commands
        .spawn((
            Text::new(
                "Each service uses your own account: paste the API key from its \
                 dashboard (meshy.ai, platform.tripo3d.ai, fal.ai). One fal.ai \
                 key covers both Hunyuan3D and TRELLIS 2. Keys are stored in \
                 your user config folder, never in the project, and are sent \
                 only to the service they belong to. Generations spend that \
                 account's credits, and each service's terms decide whether \
                 the output may be used commercially. The local server is \
                 Hunyuan3D 2.1's api_server.py and needs no key.",
            ),
            ui_font(&fonts.ui, 10.0),
            TextColor(rgb(text_muted())),
            Node {
                margin: UiRect::top(Val::Px(4.0)),
                ..default()
            },
        ))
        .id();

    commands
        .entity(col)
        .add_children(&[meshy, tripo, fal, local, note]);
    col
}

fn key_row(commands: &mut Commands, fonts: &EmberFonts, label: &str, slot: KeySlot) -> Entity {
    let row = row(commands, fonts, label);
    let input = password_input(commands, &fonts.ui, "API key", "");
    grow(commands, input);
    bind_text_input(
        commands,
        input,
        move |rx| {
            rx.get_resource::<Settings>()
                .map(|s| s.key(slot).to_string())
                .unwrap_or_default()
        },
        move |world, value| {
            if let Some(mut s) = world.get_resource_mut::<Settings>() {
                // Trimmed: a key pasted from a dashboard often brings a trailing
                // newline, and the provider answers that with a bare 401.
                let value = value.trim();
                if s.key(slot) != value {
                    *s.key_mut(slot) = value.to_string();
                }
            }
        },
    );
    commands.entity(row).add_child(input);
    row
}

fn row(commands: &mut Commands, fonts: &EmberFonts, label: &str) -> Entity {
    let row = commands
        .spawn(Node {
            flex_direction: FlexDirection::Row,
            align_items: AlignItems::Center,
            column_gap: Val::Px(10.0),
            width: Val::Percent(100.0),
            ..default()
        })
        .id();
    let label = commands
        .spawn((
            Text::new(label),
            ui_font(&fonts.ui, 11.0),
            TextColor(rgb(text_muted())),
            Node {
                width: Val::Px(90.0),
                flex_shrink: 0.0,
                ..default()
            },
        ))
        .id();
    commands.entity(row).add_child(label);
    row
}

/// Let a widget take the row's spare width. Edits the `Node` in place, since
/// replacing it would wipe the widget's own padding and border.
pub fn grow(commands: &mut Commands, entity: Entity) {
    commands.entity(entity).queue(|mut e: EntityWorldMut| {
        if let Some(mut node) = e.get_mut::<Node>() {
            node.flex_grow = 1.0;
        }
    });
}
