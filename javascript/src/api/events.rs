//! Actions, broadcast events, RPCs, HTTP, JSON and the network status.
//!
//! The functions that take arguments (`action`, `action_on`, `emit`, `rpc`)
//! take them as one object, `emit("boss_died", {boss: "ogre"})`, as Lua takes a
//! table. A nested object or array in it travels as JSON text (see
//! [`convert::to_action_value`]).

use boa_engine::{Context, JsObject, JsValue};
use renzora::{ScriptActionValue as ActionValue, ScriptCommand};

use super::args::Args;
use super::{add, command};
use crate::convert;

pub fn register(ctx: &mut Context, m: &JsObject) {
    // action("name", {key: value}): a ScriptAction a domain crate or plugin
    // handles.
    command!(ctx, m, "action", |a, c| ScriptCommand::Action {
        name: a.str(0, "name")?,
        target_entity: None,
        args: convert::action_args("action", a.get(1), c)?,
    });
    // action_on("EntityName", "name", {key: value}): the same, aimed at
    // another entity.
    command!(ctx, m, "action_on", |a, c| ScriptCommand::Action {
        target_entity: Some(a.str(0, "entity")?),
        name: a.str(1, "name")?,
        args: convert::action_args("action_on", a.get(2), c)?,
    });
    // emit("name", {key: value}): every script's on_event(name, args) fires
    // next frame, as do Rust observers of `renzora::GameEvent`. Use it when the
    // sender should not have to know who is listening ("the boss died"); use
    // set_on / get_on when you know which entity you mean. Delivery is next
    // frame, so a script never sees its own emit in the same hook.
    command!(ctx, m, "emit", |a, c| ScriptCommand::Emit {
        name: a.str(0, "name")?,
        args: convert::action_args("emit", a.get(1), c)?,
    });
    // rpc("name", {key: value}): a networked RPC. Sent as a `net_rpc` action
    // carrying the RPC name in the reserved `__rpc` key, which must match
    // `renzora_network::rpc::RPC_NAME_KEY`; remote peers receive it in
    // on_rpc(name, args, sender).
    command!(ctx, m, "rpc", |a, c| {
        let name = a.str(0, "name")?;
        let mut args = convert::action_args("rpc", a.get(1), c)?;
        args.push(("__rpc".to_string(), ActionValue::String(name)));
        ScriptCommand::Action {
            name: "net_rpc".to_string(),
            target_entity: None,
            args,
        }
    });

    // -- HTTP (asynchronous) --
    // http_get(url, callback = "get"): the response arrives as
    // on_http(callback, status, body) a frame or more later.
    command!(ctx, m, "http_get", |a, c| ScriptCommand::HttpRequest {
        method: "GET".into(),
        url: a.str(0, "url")?,
        body: None,
        callback: a.opt_str(1, "callback")?.unwrap_or_else(|| "get".into()),
    });
    // http_post(url, body, callback = "post"). `body` is sent as given; an
    // object or array is JSON-encoded first, which is nearly always what was
    // meant.
    command!(ctx, m, "http_post", |a, c| {
        let url = a.str(0, "url")?;
        let body = match a.get(1) {
            Some(v) => match convert::string(v) {
                Some(s) => s,
                None => convert::to_json(v, c, 0)?.to_string(),
            },
            None => String::new(),
        };
        ScriptCommand::HttpRequest {
            method: "POST".into(),
            url,
            body: Some(body),
            callback: a.opt_str(2, "callback")?.unwrap_or_else(|| "post".into()),
        }
    });

    // -- JSON --
    // json_parse(text): objects / arrays / strings / numbers / booleans /
    // null, or null when the text is not JSON, as the Lua version returns nil.
    // `JSON.parse`, which throws instead, is there too.
    add(ctx, m, "json_parse", |_, args, ctx| {
        let text = Args::new(args, "json_parse").str(0, "text")?;
        match serde_json::from_str::<serde_json::Value>(&text) {
            Ok(v) => convert::from_json(ctx, &v),
            Err(_) => Ok(JsValue::null()),
        }
    });
    // json_encode(value, pretty = false): the inverse. Throws a TypeError for
    // a value JSON cannot hold, rather than writing something that will not
    // parse back.
    add(ctx, m, "json_encode", |_, args, ctx| {
        let a = Args::new(args, "json_encode");
        let value = a.raw(0);
        let json = convert::to_json(&value, ctx, 0)?;
        let pretty = a.opt_bool(1).unwrap_or(false);
        let text = if pretty {
            serde_json::to_string_pretty(&json).unwrap_or_default()
        } else {
            json.to_string()
        };
        Ok(convert::text(&text))
    });

    // -- Network status --
    // net_is_server(): true on the dedicated or host server. Gate
    // server-authoritative logic on it so it does not also run on clients.
    add(ctx, m, "net_is_server", |_, _, _| {
        Ok(JsValue::from(crate::frame::with(|c| c.net_is_server).unwrap_or(false)))
    });
    // net_is_client(): networking is active and this is not the server.
    add(ctx, m, "net_is_client", |_, _, _| {
        Ok(JsValue::from(
            crate::frame::with(|c| c.net_is_connected && !c.net_is_server).unwrap_or(false),
        ))
    });
    // net_is_connected(): connected to a server (client) or running (server).
    add(ctx, m, "net_is_connected", |_, _, _| {
        Ok(JsValue::from(crate::frame::with(|c| c.net_is_connected).unwrap_or(false)))
    });
    // net_player_count(): connected clients (server only; 0 elsewhere).
    add(ctx, m, "net_player_count", |_, _, _| {
        Ok(JsValue::from(crate::frame::with(|c| c.net_player_count as f64).unwrap_or(0.0)))
    });
}
