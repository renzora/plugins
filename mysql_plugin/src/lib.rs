//! MySQL for a game: queries from scripts, entities to and from rows, and the
//! game's save slots.
//!
//! # Scripts
//!
//! `script.rs` declares `mysql_query`, `mysql_query_with`, `mysql_transaction`,
//! `mysql_spawn`, `mysql_save_entity` and `mysql_load_entity` through the
//! engine's `ScriptExtension`, so every script language gets them, and each is
//! a `ScriptAction` a Rust script sends directly. Any SQL the server accepts
//! goes through. The SQL runs on worker threads (`query.rs`) and the answer
//! comes back as a game event the script named. `entities.rs` turns rows into
//! entities and entity trees into rows.
//!
//! # Saves
//!
//! Installs a [`renzora::save::SaveBackend`] when a `mysql.toml` is found that
//! leaves `saves` on (never on the defaults alone), so every slot call a game already
//! makes (`save_write`, `save_load`, `save_exists`, `save_delete`, `save_slots`,
//! from Lua or from a Rust script through `renzora::save`) lands in a MySQL table
//! instead of a `.json` file. A game's scripts do not change to use it, and do
//! not change back if it is removed: take the plugin out and the same calls go
//! to the save folder again.
//!
//! What it stores is the same text a slot file holds, so the layout version and
//! the "made by a newer version" refusal still come from the engine, and a slot
//! copied out of the `data` column is a valid `<slot>.json`.
//!
//! `Runtime` scope, since the game is the thing that saves. In the editor it
//! is installed too but does nothing until something saves: Play runs the
//! game as its own process, which loads this plugin for itself, so a playtest
//! saves to the same table the shipped game will.
//!
//! The MySQL half (`config.rs`, `db.rs`) is plain Rust with no Bevy and no
//! engine types, so it can be compiled and tried against a server on its own;
//! this file is only the glue between it and the engine.

mod config;
mod db;
mod entities;
mod query;
mod script;

use std::path::PathBuf;
use std::sync::Arc;

use bevy::prelude::*;
use renzora::save::{SaveBackend, SaveError};
use renzora::CurrentProject;

use db::{Database, DbError, Note};

#[derive(Default)]
pub struct MySqlPlugin;

/// The database the installed backend talks to, kept so the project-change
/// system can tell it which game is running and where the config now says to
/// connect.
#[derive(Resource)]
struct MySqlSaves(Arc<Database>);

impl Plugin for MySqlPlugin {
    fn build(&self, app: &mut App) {
        // Read now, from beside the executable, so the backend exists before
        // anything can save. The project folder's file is only known once the
        // project loads, and `follow_project` reads it then.
        let config = match config::find(exe_dir().as_deref(), None) {
            Ok((config, _)) => config,
            Err(e) => {
                // A broken file beside the game is refused outright rather
                // than replaced with the defaults; see `config::find`.
                error!("[mysql] {e}; saves stay in the save folder");
                return;
            }
        };
        let workers = query::Workers::start(config.clone());
        let allow_clients = config.allow_clients;
        // The save slots are claimed in `follow_project`, not here: only the
        // project's own file says whether this plugin keeps them, and nothing
        // can save before then anyway (there is no game name). Claiming on the
        // defaults here once made another database plugin step aside, after
        // which this one read `saves = false` and let go, leaving nobody.
        let database = Arc::new(Database::new(config, log_note));
        app.insert_resource(MySqlSaves(database));

        app.register_type::<entities::MySqlRow>();
        app.insert_resource(script::MySqlScripts::new(workers, allow_clients));
        app.world_mut()
            .get_resource_or_insert_with(renzora::script_extension::ScriptExtensions::default)
            .register(script::MySqlExtension);
        app.add_observer(script::on_action);
        app.add_systems(Update, script::deliver);
        app.add_systems(
            Update,
            follow_project.run_if(resource_exists_and_changed::<CurrentProject>),
        );
    }
}

/// Name the game and re-read the config now the project folder is known.
///
/// Follows `CurrentProject` rather than running once, as the engine's own
/// save-folder system does, so switching project in the editor switches whose
/// slots are used.
fn follow_project(project: Res<CurrentProject>, saves: Res<MySqlSaves>, mut scripts: ResMut<script::MySqlScripts>) {
    match config::find(exe_dir().as_deref(), Some(project.path.as_path())) {
        Ok((config, source)) => {
            // The save slots only on request: a `mysql.toml` that leaves
            // `saves` on. Without one, the project never asked for a database,
            // and the engine's own save files are what it expects; claiming
            // them on the defaults sent every project in an editor with this
            // plugin installed to a server that was usually not running, and
            // each save waited out the connection timeout and then failed.
            let keeps_saves = config.saves && matches!(source, config::Source::File(_));
            info!(
                "[mysql] \"{}\" uses {}{}, from {source}",
                project.config.name,
                config::redacted(&config.url),
                if keeps_saves { format!(", saves in table `{}`", config.table) } else { String::new() },
            );
            // The project's file can turn saves off (or on) after the backend
            // was decided from the executable's folder alone.
            if keeps_saves {
                claim_saves(&saves.0);
            } else if renzora::save::backend_name().as_deref() == Some(BACKEND) {
                renzora::save::set_backend(None);
            }
            scripts.allow_clients = config.allow_clients;
            scripts.workers.set_config(config.clone());
            saves.0.set_config(config);
        }
        // Keep the config the backend already has: the error names the file,
        // and saving somewhere known beats not saving.
        Err(e) => error!("[mysql] {e}; keeping the previous MySQL settings"),
    }
    saves.0.set_game(&project.config.name);
}

/// The save backend's name, which is how another installed database plugin
/// can tell the slots are taken.
const BACKEND: &str = "MySQL";

/// Keep the save slots here, unless another plugin already keeps them. Both
/// database plugins installed with saves on would otherwise hand the slots to
/// whichever happened to load last, silently, and a game would save to one
/// database on one launch and look in the other on the next.
fn claim_saves(database: &Arc<Database>) {
    match renzora::save::backend_name() {
        Some(other) if other != BACKEND => warn!(
            "[mysql] saves = true, but the {other} plugin already keeps the save slots, so they stay \
             there. Set saves = false in one of the two plugins' config files"
        ),
        _ => renzora::save::set_backend(Some(Box::new(MySqlBackend(database.clone())))),
    }
}

struct MySqlBackend(Arc<Database>);

impl SaveBackend for MySqlBackend {
    fn name(&self) -> &str {
        BACKEND
    }

    fn write(&self, slot: &str, text: &str) -> Result<(), SaveError> {
        self.0.write(slot, text).map_err(save_error)
    }

    fn read(&self, slot: &str) -> Result<Option<String>, SaveError> {
        self.0.read(slot).map_err(save_error)
    }

    fn delete(&self, slot: &str) -> Result<(), SaveError> {
        self.0.delete(slot).map_err(save_error)
    }

    fn slots(&self) -> Result<Vec<String>, SaveError> {
        self.0.slots().map_err(save_error)
    }

    fn exists(&self, slot: &str) -> Result<bool, SaveError> {
        self.0.exists(slot).map_err(save_error)
    }
}

fn save_error(e: DbError) -> SaveError {
    match e {
        DbError::NoGame => SaveError::NoLocation,
        DbError::Failed(why) => SaveError::Io(why),
    }
}

fn log_note(note: Note) {
    match note {
        Note::Connected { url, table } => info!("[mysql] connected to {url}, table `{table}`"),
        Note::Unreachable(why) => warn!(
            "[mysql] {why}. Saving and loading fail at once until it is reachable; \
             checking every {}s",
            db::RETRY_AFTER.as_secs()
        ),
        Note::Reconnected { url, table } => info!("[mysql] {url} answers again; saves in table `{table}`"),
    }
}

fn exe_dir() -> Option<PathBuf> {
    std::env::current_exe().ok()?.parent().map(PathBuf::from)
}

renzora::plugin!(MySqlPlugin, Runtime);
