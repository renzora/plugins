//! Saved games (`save_*`), through `renzora::save`.
//!
//! Called straight into the contract crate rather than sent as commands, for
//! the Lua plugin's reason: a save is read back in the same hook that asks for
//! it (`save_load("slot1")` then `save_get("level")`), and a command would not
//! land until the hook had returned.
//!
//! The slot functions return `{ok, error}`, an object standing in for Lua's two
//! return values: `{ok: true, error: null}` on success, `{ok: false, error:
//! "why"}` on failure. An object rather than an array because that is how a
//! JavaScript API reports an outcome, and `const {ok, error} = save_write(..)`
//! reads as what it means.

use boa_engine::{Context, JsNativeError, JsObject, JsResult, JsValue};
use renzora::save;

use super::add;
use super::args::{type_name, Args};
use crate::convert;

fn outcome(ctx: &mut Context, result: Result<(), save::SaveError>) -> JsResult<JsValue> {
    let (ok, why) = match result {
        Ok(()) => (true, None),
        Err(e) => (false, Some(e.to_string())),
    };
    let obj = JsObject::with_object_proto(ctx.intrinsics());
    obj.create_data_property_or_throw(convert::key("ok"), ok, ctx)?;
    obj.create_data_property_or_throw(convert::key("error"), convert::opt_text(why.as_deref()), ctx)?;
    Ok(obj.into())
}

pub fn register(ctx: &mut Context, m: &JsObject) {
    add(ctx, m, "save_set", |_, args, ctx| {
        let a = Args::new(args, "save_set");
        let key = a.str(0, "key")?;
        let value = a.raw(1);
        if value.is_undefined() && a.len() < 2 {
            return Err(a.missing("value"));
        }
        let Some(saved) = convert::to_saved_value(&value, ctx) else {
            return Err(JsNativeError::typ()
                .with_message(format!(
                    "save_set: a {} cannot be saved; use a number, boolean, string, Vec2, Vec3 or Color",
                    type_name(&value)
                ))
                .into());
        };
        save::set(key, saved);
        Ok(JsValue::undefined())
    });
    // `save_get(key, fallback = null)`.
    add(ctx, m, "save_get", |_, args, ctx| {
        let a = Args::new(args, "save_get");
        let key = a.str(0, "key")?;
        match save::get(&key) {
            Some(v) => convert::from_script_value(ctx, &v),
            None => Ok(a.get(1).cloned().unwrap_or_else(JsValue::null)),
        }
    });
    add(ctx, m, "save_has", |_, args, _| {
        let key = Args::new(args, "save_has").str(0, "key")?;
        Ok(JsValue::from(save::get(&key).is_some()))
    });
    add(ctx, m, "save_remove", |_, args, _| {
        let key = Args::new(args, "save_remove").str(0, "key")?;
        Ok(JsValue::from(save::remove(&key)))
    });
    add(ctx, m, "save_clear", |_, _, _| {
        save::clear();
        Ok(JsValue::undefined())
    });
    add(ctx, m, "save_keys", |_, _, ctx| Ok(convert::text_list(ctx, &save::keys())));
    add(ctx, m, "save_write", |_, args, ctx| {
        let slot = Args::new(args, "save_write").str(0, "slot")?;
        outcome(ctx, save::write(&slot))
    });
    add(ctx, m, "save_load", |_, args, ctx| {
        let slot = Args::new(args, "save_load").str(0, "slot")?;
        outcome(ctx, save::load(&slot))
    });
    add(ctx, m, "save_delete", |_, args, ctx| {
        let slot = Args::new(args, "save_delete").str(0, "slot")?;
        outcome(ctx, save::delete(&slot))
    });
    add(ctx, m, "save_exists", |_, args, _| {
        let slot = Args::new(args, "save_exists").str(0, "slot")?;
        Ok(JsValue::from(save::exists(&slot)))
    });
    // An array of `{name: "slot1", saved_at: 1730000000}`, newest first.
    add(ctx, m, "save_slots", |_, _, ctx| {
        let mut list = Vec::new();
        for slot in save::list() {
            let entry = JsObject::with_object_proto(ctx.intrinsics());
            entry.create_data_property_or_throw(convert::key("name"), convert::text(&slot.name), ctx)?;
            entry.create_data_property_or_throw(convert::key("saved_at"), slot.saved_at as f64, ctx)?;
            list.push(JsValue::from(entry));
        }
        Ok(boa_engine::object::builtins::JsArray::from_iter(list, ctx).into())
    });
}
