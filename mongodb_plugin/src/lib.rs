//! MongoDB for a game: queries from scripts, entities to and from documents,
//! and the game's save slots.
//!
//! # Scripts
//!
//! `script.rs` declares `mongo_find`, `mongo_find_with`, `mongo_insert`,
//! `mongo_update`, `mongo_delete`, `mongo_aggregate`, `mongo_count`,
//! `mongo_command`, `mongo_spawn`, `mongo_save_entity` and `mongo_load_entity`
//! through the engine's `ScriptExtension`, so every script language gets them,
//! and each is a `ScriptAction` a Rust script sends directly. `mongo_command`
//! runs any database command, so nothing MongoDB can do is out of reach. The
//! work runs on worker threads (`query.rs`); the answer comes back as a game
//! event the script named. `entities.rs` turns documents into entities and
//! entity trees into documents.
//!
//! # Saves
//!
//! Unless `saves = false` in `mongodb.toml`, installs a
//! [`renzora::save::SaveBackend`] so `save_write`, `save_load` and the rest go
//! to a collection (`db.rs`). Remove the plugin and they go back to files.
//!
//! `Runtime` scope: the game is what queries and saves, and Play runs the game
//! as its own process, which loads this plugin for itself.

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

use db::{Database, DbError};

#[derive(Default)]
pub struct MongoPlugin;

/// The database the save backend talks to, kept so the project-change system
/// can name the game and pass on a re-read config.
#[derive(Resource)]
struct MongoSaves(Arc<Database>);

impl Plugin for MongoPlugin {
    fn build(&self, app: &mut App) {
        // From beside the executable, so the backend exists before anything
        // can save; the project's file is read once the project loads.
        let config = match config::find(exe_dir().as_deref(), None) {
            Ok((config, _)) => config,
            Err(e) => {
                error!("[mongodb] {e}; saves stay in the save folder");
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
        let database = Arc::new(Database::new(config, |line| info!("[mongodb] {line}")));
        app.insert_resource(MongoSaves(database));

        app.register_type::<entities::MongoDoc>();
        app.insert_resource(script::MongoScripts::new(workers, allow_clients));
        app.world_mut()
            .get_resource_or_insert_with(renzora::script_extension::ScriptExtensions::default)
            .register(script::MongoExtension);
        app.add_observer(script::on_action);
        app.add_systems(Update, script::deliver);
        app.add_systems(Update, follow_project.run_if(resource_exists_and_changed::<CurrentProject>));
    }
}

/// Name the game and re-read the config now the project folder is known.
fn follow_project(project: Res<CurrentProject>, saves: Res<MongoSaves>, mut scripts: ResMut<script::MongoScripts>) {
    match config::find(exe_dir().as_deref(), Some(project.path.as_path())) {
        Ok((config, source)) => {
            info!(
                "[mongodb] \"{}\" uses {} / {}{}, from {source}",
                project.config.name,
                config::redacted(&config.url),
                config.database,
                if config.saves { format!(", saves in `{}`", config.collection) } else { String::new() },
            );
            if config.saves {
                claim_saves(&saves.0);
            } else if renzora::save::backend_name().as_deref() == Some(BACKEND) {
                renzora::save::set_backend(None);
            }
            scripts.allow_clients = config.allow_clients;
            scripts.workers.set_config(config.clone());
            saves.0.set_config(config);
        }
        Err(e) => error!("[mongodb] {e}; keeping the previous MongoDB settings"),
    }
    saves.0.set_game(&project.config.name);
}

/// The save backend's name, which is how another installed database plugin
/// can tell the slots are taken.
const BACKEND: &str = "MongoDB";

/// Keep the save slots here, unless another plugin already keeps them. Both
/// database plugins installed with saves on would otherwise hand the slots to
/// whichever happened to load last, silently, and a game would save to one
/// database on one launch and look in the other on the next.
fn claim_saves(database: &Arc<Database>) {
    match renzora::save::backend_name() {
        Some(other) if other != BACKEND => warn!(
            "[mongodb] saves = true, but the {other} plugin already keeps the save slots, so they stay \
             there. Set saves = false in one of the two plugins' config files"
        ),
        _ => renzora::save::set_backend(Some(Box::new(MongoBackend(database.clone())))),
    }
}

struct MongoBackend(Arc<Database>);

impl SaveBackend for MongoBackend {
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

fn exe_dir() -> Option<PathBuf> {
    std::env::current_exe().ok()?.parent().map(PathBuf::from)
}

renzora::plugin!(MongoPlugin, Runtime);
