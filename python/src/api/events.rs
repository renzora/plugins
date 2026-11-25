//! Actions, broadcast events, RPCs, HTTP, JSON and the network status.
//!
//! The functions that take an argument dict (`action`, `action_on`, `emit`,
//! `rpc`) also take keyword arguments, merged over the dict:
//! `emit("boss_died", {"boss": "ogre"})` and `emit("boss_died", boss="ogre")`
//! send the same event.

use renzora::{ScriptActionValue as ActionValue, ScriptCommand};
use rustpython_vm::builtins::PyDict;
use rustpython_vm::function::FuncArgs;
use rustpython_vm::{Py, PyResult, VirtualMachine};

use super::args::Args;
use super::{add, command};
use crate::convert;

pub fn register(vm: &VirtualMachine, m: &Py<PyDict>) {
    // action("name", {key: value}, **kwargs): a ScriptAction a domain crate or
    // plugin handles.
    command!(vm, m, "action", |a| ScriptCommand::Action {
        name: a.str(0, "name")?,
        target_entity: None,
        args: convert::action_args(a.get(1), a.raw(), a.vm())?,
    });
    // action_on("EntityName", "name", {key: value}, **kwargs): the same,
    // aimed at another entity.
    command!(vm, m, "action_on", |a| ScriptCommand::Action {
        target_entity: Some(a.str(0, "entity")?),
        name: a.str(1, "name")?,
        args: convert::action_args(a.get(2), a.raw(), a.vm())?,
    });
    // emit("name", {key: value}, **kwargs): every script's on_event(name, args)
    // fires next frame, as do Rust observers of `renzora::GameEvent`. Use it
    // when the sender should not have to know who is listening ("the boss
    // died"); use set_on / get_on when you know which entity you mean.
    // Delivery is next frame, so a script never sees its own emit in the same
    // hook.
    command!(vm, m, "emit", |a| ScriptCommand::Emit {
        name: a.str(0, "name")?,
        args: convert::action_args(a.get(1), a.raw(), a.vm())?,
    });
    // rpc("name", {key: value}, **kwargs): a networked RPC. Sent as a `net_rpc`
    // action carrying the RPC name in the reserved `__rpc` key, which must match
    // `renzora_network::rpc::RPC_NAME_KEY`; remote peers receive it in
    // on_rpc(name, args, sender).
    command!(vm, m, "rpc", |a| {
        let name = a.str(0, "name")?;
        let mut args = convert::action_args(a.get(1), a.raw(), a.vm())?;
        args.push(("__rpc".to_string(), ActionValue::String(name)));
        ScriptCommand::Action {
            name: "net_rpc".to_string(),
            target_entity: None,
            args,
        }
    });

    // -- HTTP (asynchronous) --
    // http_get(url, callback="get"): the response arrives as
    // on_http(callback, status, body) a frame or more later.
    command!(vm, m, "http_get", |a| ScriptCommand::HttpRequest {
        method: "GET".into(),
        url: a.str(0, "url")?,
        body: None,
        callback: a.opt_str(1, "callback")?.unwrap_or_else(|| "get".into()),
    });
    // http_post(url, body, callback="post"). `body` is sent as given; a dict
    // or list is JSON-encoded first, which is nearly always what was meant.
    add(vm, m, "http_post", |args: FuncArgs, vm: &VirtualMachine| -> PyResult<()> {
        let a = Args::new(&args, vm, "http_post");
        let url = a.str(0, "url")?;
        let body = match a.get(1) {
            Some(v) => match convert::string(v) {
                Some(s) => s,
                None => convert::to_json(v, vm, 0)?.to_string(),
            },
            None => String::new(),
        };
        crate::buffers::push_command(ScriptCommand::HttpRequest {
            method: "POST".into(),
            url,
            body: Some(body),
            callback: a.opt_str(2, "callback")?.unwrap_or_else(|| "post".into()),
        });
        Ok(())
    });

    // -- JSON --
    // json_parse(text): dict / list / str / int / float / bool / None, or None
    // when the text is not JSON, as the Lua version returns nil.
    add(vm, m, "json_parse", |args: FuncArgs, vm: &VirtualMachine| -> PyResult {
        let text = Args::new(&args, vm, "json_parse").str(0, "text")?;
        match serde_json::from_str::<serde_json::Value>(&text) {
            Ok(v) => convert::from_json(vm, &v),
            Err(_) => Ok(vm.ctx.none()),
        }
    });
    // json_encode(value, pretty=False): the inverse. Raises TypeError for a
    // value JSON cannot hold, rather than writing something that will not
    // parse back.
    add(vm, m, "json_encode", |args: FuncArgs, vm: &VirtualMachine| -> PyResult<String> {
        let a = Args::new(&args, vm, "json_encode");
        let value = args.args.first().cloned().unwrap_or_else(|| vm.ctx.none());
        let json = convert::to_json(&value, vm, 0)?;
        let pretty = a.opt_bool(1)?.unwrap_or(false);
        Ok(if pretty {
            serde_json::to_string_pretty(&json).unwrap_or_default()
        } else {
            json.to_string()
        })
    });

    // -- Network status --
    // net_is_server(): true on the dedicated or host server. Gate
    // server-authoritative logic on it so it does not also run on clients.
    add(vm, m, "net_is_server", |_args: FuncArgs, _vm: &VirtualMachine| -> PyResult<bool> {
        Ok(crate::frame::with(|c| c.net_is_server).unwrap_or(false))
    });
    // net_is_client(): networking is active and this is not the server.
    add(vm, m, "net_is_client", |_args: FuncArgs, _vm: &VirtualMachine| -> PyResult<bool> {
        Ok(crate::frame::with(|c| c.net_is_connected && !c.net_is_server).unwrap_or(false))
    });
    // net_is_connected(): connected to a server (client) or running (server).
    add(vm, m, "net_is_connected", |_args: FuncArgs, _vm: &VirtualMachine| -> PyResult<bool> {
        Ok(crate::frame::with(|c| c.net_is_connected).unwrap_or(false))
    });
    // net_player_count(): connected clients (server only; 0 elsewhere).
    add(vm, m, "net_player_count", |_args: FuncArgs, _vm: &VirtualMachine| -> PyResult<i64> {
        Ok(crate::frame::with(|c| c.net_player_count as i64).unwrap_or(0))
    });
}
