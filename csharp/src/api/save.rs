//! Saved games (`save_*`), through `renzora::save`.
//!
//! Called straight into the contract crate rather than sent as commands, for
//! the Lua plugin's reason: a save is read back in the same hook that asks for
//! it (`SaveLoad("slot1")` then `SaveGet("level")`), and a command would not
//! land until the hook had returned.
//!
//! The slot functions return `{ok, error}`, which the C# side turns into a
//! `SaveResult` record.

use renzora::save;
use renzora::serde_json::{Map, Value};

use super::{Args, Out};
use crate::convert;

fn outcome(result: Result<(), save::SaveError>) -> Value {
    let mut m = Map::new();
    match result {
        Ok(()) => {
            m.insert("ok".into(), Value::Bool(true));
            m.insert("error".into(), Value::Null);
        }
        Err(e) => {
            m.insert("ok".into(), Value::Bool(false));
            m.insert("error".into(), Value::String(e.to_string()));
        }
    }
    Value::Object(m)
}

pub fn call(name: &str, a: &Args) -> Option<Out> {
    Some(match name {
        "save_set" => (|| {
            let key = a.str(0, "key")?;
            if a.len() < 2 {
                return Err(a.missing("value"));
            }
            let value = a.raw(1);
            let Some(saved) = convert::to_saved_value(&value) else {
                return Err(format!(
                    "save_set(): a {} cannot be saved; use a number, bool, string, Vector2, Vector3 or Color",
                    convert::type_name(&value)
                ));
            };
            save::set(key, saved);
            Ok(Value::Null)
        })(),
        // `save_get(key, fallback = null)`.
        "save_get" => a.str(0, "key").map(|key| match save::get(&key) {
            Some(v) => convert::from_script_value(&v),
            None => a.raw(1),
        }),
        "save_has" => a.str(0, "key").map(|key| Value::Bool(save::get(&key).is_some())),
        "save_remove" => a.str(0, "key").map(|key| Value::Bool(save::remove(&key))),
        "save_clear" => {
            save::clear();
            Ok(Value::Null)
        }
        "save_keys" => Ok(convert::text_list(&save::keys())),
        "save_write" => a.str(0, "slot").map(|slot| outcome(save::write(&slot))),
        "save_load" => a.str(0, "slot").map(|slot| outcome(save::load(&slot))),
        "save_delete" => a.str(0, "slot").map(|slot| outcome(save::delete(&slot))),
        "save_exists" => a.str(0, "slot").map(|slot| Value::Bool(save::exists(&slot))),
        // A list of `{name, saved_at}`, newest first.
        "save_slots" => Ok(Value::Array(
            save::list()
                .into_iter()
                .map(|slot| {
                    let mut m = Map::new();
                    m.insert("name".into(), Value::String(slot.name.clone()));
                    m.insert("saved_at".into(), Value::from(slot.saved_at));
                    Value::Object(m)
                })
                .collect(),
        )),
        _ => return None,
    })
}
