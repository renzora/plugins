//! Actions, broadcast events, RPCs and HTTP.
//!
//! The functions that take arguments (`action`, `action_on`, `emit`, `rpc`)
//! take them as one object: in C#, a dictionary or an anonymous object,
//! `Emit("boss_died", new { boss = "ogre" })`, which the C# side encodes as a
//! JSON object. A nested object or list in it travels as JSON text (see
//! [`convert::to_action_value`]).

use renzora::{ScriptActionValue as ActionValue, ScriptCommand};

use super::{push, Args, Out};
use crate::convert;

pub fn call(name: &str, a: &Args) -> Option<Out> {
    Some(match name {
        // action("name", args): a ScriptAction a domain crate or plugin handles.
        "action" => (|| {
            push(ScriptCommand::Action {
                name: a.str(0, "name")?,
                target_entity: None,
                args: convert::action_args("action", a.get(1))?,
            })
        })(),
        // action_on("EntityName", "name", args): aimed at another entity.
        "action_on" => (|| {
            push(ScriptCommand::Action {
                target_entity: Some(a.str(0, "entity")?),
                name: a.str(1, "name")?,
                args: convert::action_args("action_on", a.get(2))?,
            })
        })(),
        // emit("name", args): every script's OnEvent(name, args) fires next
        // frame, as do Rust observers of `renzora::GameEvent`.
        "emit" => (|| {
            push(ScriptCommand::Emit {
                name: a.str(0, "name")?,
                args: convert::action_args("emit", a.get(1))?,
            })
        })(),
        // rpc("name", args): sent as a `net_rpc` action carrying the RPC name in
        // the reserved `__rpc` key, which must match
        // `renzora_network::rpc::RPC_NAME_KEY`; remote peers receive it in
        // OnRpc(name, args, sender).
        "rpc" => (|| {
            let rpc = a.str(0, "name")?;
            let mut args = convert::action_args("rpc", a.get(1))?;
            args.push(("__rpc".to_string(), ActionValue::String(rpc)));
            push(ScriptCommand::Action {
                name: "net_rpc".to_string(),
                target_entity: None,
                args,
            })
        })(),

        // http_get(url, callback = "get"): the response arrives as
        // OnHttp(callback, status, body) a frame or more later.
        "http_get" => (|| {
            push(ScriptCommand::HttpRequest {
                method: "GET".into(),
                url: a.str(0, "url")?,
                body: None,
                callback: a.opt_str(1, "callback")?.unwrap_or_else(|| "get".into()),
            })
        })(),
        // http_post(url, body, callback = "post"). A body that is not a string
        // is sent as its JSON, which is nearly always what was meant.
        "http_post" => (|| {
            let url = a.str(0, "url")?;
            let body = a.get(1).map(convert::display).unwrap_or_default();
            push(ScriptCommand::HttpRequest {
                method: "POST".into(),
                url,
                body: Some(body),
                callback: a.opt_str(2, "callback")?.unwrap_or_else(|| "post".into()),
            })
        })(),
        _ => return None,
    })
}
