//! Functions other crates and plugins declare through `ScriptExtension`.
//!
//! `apply_force`, `nav_set_destination`, `tr`, the MySQL plugin's
//! `mysql_query(sql, callback)`: none of them is written for C#. Each is a
//! [`Binding`] (a name, a kind, typed parameters) that every language backend
//! builds a function from. In C# that function is `Call("mysql_query", sql,
//! "answer")`, which reaches [`call`] by name; the answer to a query arrives
//! later as `OnEvent`, as it does in every language.
//!
//! The table is process-wide rather than thread-local: a binding list is the
//! same for every thread, it changes once (when the extensions finish
//! registering, before the first frame), and the backend refreshes it from the
//! hook's context exactly as the JavaScript plugin does.

use std::collections::HashMap;
use std::sync::{Arc, Mutex};

use renzora::script_extension::{substitute, Binding, BindingKind, ParamKind};
use renzora::serde_json::Value;
use renzora::{get_handler as host, ScriptActionValue as ActionValue, ScriptCommand};

use crate::api::Out;
use crate::buffers::push_command;
use crate::convert;

static TABLE: Mutex<Option<Arc<HashMap<String, Binding>>>> = Mutex::new(None);

/// Replace the table with `bindings`.
pub fn install(bindings: &[Binding]) {
    let map: HashMap<String, Binding> = bindings.iter().map(|b| (b.name.clone(), b.clone())).collect();
    if let Ok(mut t) = TABLE.lock() {
        *t = Some(Arc::new(map));
    }
}

/// Run the binding called `name`, or `None` when no plugin declares one.
pub fn call(name: &str, args: &[Value]) -> Option<Out> {
    let table = TABLE.lock().ok()?.clone()?;
    let b = table.get(name)?;
    Some(run(b, args))
}

fn run(b: &Binding, args: &[Value]) -> Out {
    match &b.kind {
        BindingKind::Action { action } => {
            let mut packed = Vec::with_capacity(b.params.len());
            let mut i = 0;
            for p in &b.params {
                let (value, used) = arg_value(args, i, p.kind);
                packed.push((p.name.clone(), value));
                i += used;
            }
            push_command(ScriptCommand::Action {
                name: action.clone(),
                target_entity: None,
                args: packed,
            });
            Ok(Value::Null)
        }
        BindingKind::Read { component, field } => {
            // Placeholders are positional over the SCRIPT arguments, not the
            // packed ones, so a `Vec3` parameter does not shift the numbering.
            let subs: Vec<String> = (0..b.params.len().max(args.len()))
                .map(|i| args.get(i).map(convert::display).unwrap_or_default())
                .collect();
            let component = substitute(component, &subs);
            let field = substitute(field, &subs);
            Ok(host::call_get(None, &component, &field)
                .map(convert::from_property_value)
                .unwrap_or(Value::Null))
        }
        BindingKind::Translate => {
            let key = args.first().map(convert::display).unwrap_or_default();
            Ok(Value::String(renzora::lang::t(&key)))
        }
        // A kind added to the contract after this plugin was written: say so
        // rather than doing nothing silently.
        #[allow(unreachable_patterns)]
        _ => Err(format!("`{}` is a kind of script function this plugin does not know", b.name)),
    }
}

/// One declared parameter from the script's arguments, and how many arguments
/// it consumed.
///
/// A missing or wrong-typed argument becomes the type's zero rather than an
/// error, matching the other plugins: scripts depend on `apply_force(1, 2)`
/// meaning a `z` of zero. As in JavaScript, a `Vec3` parameter takes one vector
/// as well as three numbers, and a `Str` parameter given a list or an object
/// receives its JSON text, so `Call("mysql_query_with", sql, new[] { 3 },
/// "scores")` needs no encoding by hand.
fn arg_value(args: &[Value], i: usize, kind: ParamKind) -> (ActionValue, usize) {
    let v = args.get(i).filter(|v| !v.is_null());
    let f = |v: Option<&Value>| v.and_then(convert::number).unwrap_or(0.0) as f32;
    match kind {
        ParamKind::Float => (ActionValue::Float(f(v)), 1),
        ParamKind::Int => (ActionValue::Int(v.and_then(convert::whole).unwrap_or(0)), 1),
        ParamKind::Bool => (
            ActionValue::Bool(match v {
                Some(Value::Bool(b)) => *b,
                Some(Value::Number(n)) => n.as_f64().is_some_and(|n| n != 0.0),
                Some(Value::String(s)) => !s.is_empty(),
                Some(_) => true,
                None => false,
            }),
            1,
        ),
        ParamKind::Str => (ActionValue::String(v.map(convert::display).unwrap_or_default()), 1),
        ParamKind::Vec3 => {
            let as_vector = match v {
                Some(v) if convert::number(v).is_none() => convert::shape(v),
                _ => None,
            };
            match as_vector {
                Some(convert::Shape::Vec3(xyz)) => (ActionValue::Vec3(xyz), 1),
                Some(convert::Shape::Vec2([x, y])) => (ActionValue::Vec3([x, y, 0.0]), 1),
                _ => (
                    ActionValue::Vec3([f(args.get(i)), f(args.get(i + 1)), f(args.get(i + 2))]),
                    // Consumes three script arguments; see `ParamKind::arity`.
                    kind.arity(),
                ),
            }
        }
    }
}
