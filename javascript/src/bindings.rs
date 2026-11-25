//! Functions other crates and plugins declare through `ScriptExtension`.
//!
//! `apply_force`, `nav_set_destination`, `tr`, the MySQL plugin's
//! `mysql_query(sql, callback)`: none of them is written for JavaScript. Each is
//! a [`Binding`] (a name, a kind, typed parameters) that every language backend
//! builds a function from, and this is where JavaScript builds them.
//!
//! Each binding becomes one native function that carries its own `Binding`.
//! That needs Boa's `unsafe` closure constructor, whose one condition is that
//! the closure holds no JavaScript value the garbage collector would have to
//! trace; a `Binding` is names and enums, so it holds none.
//!
//! The functions live on the global object, shared by every script on the
//! thread, so a changed binding list is applied by replacing them in place: no
//! script has to be rebuilt to see a function a late-registering plugin added.

use boa_engine::object::FunctionObjectBuilder;
use boa_engine::{Context, JsNativeError, JsObject, JsResult, JsValue, NativeFunction};
use renzora::script_extension::{substitute, Binding, BindingKind, ParamKind};
use renzora::{get_handler as host, ScriptActionValue as ActionValue, ScriptCommand};

use crate::buffers::push_command;
use crate::convert;

/// JavaScript's reserved words, which cannot be called as a bare name. A
/// binding named one of these is reachable as `renzora.<name>` only.
const RESERVED: &[&str] = &[
    "await", "break", "case", "catch", "class", "const", "continue", "debugger", "default",
    "delete", "do", "else", "enum", "export", "extends", "false", "finally", "for",
    "function", "if", "implements", "import", "in", "instanceof", "interface", "let", "new",
    "null", "package", "private", "protected", "public", "return", "static", "super",
    "switch", "this", "throw", "true", "try", "typeof", "var", "void", "while", "with",
    "yield",
];

fn is_identifier(name: &str) -> bool {
    let mut chars = name.chars();
    matches!(chars.next(), Some(c) if c == '_' || c == '$' || c.is_ascii_alphabetic())
        && chars.all(|c| c == '_' || c == '$' || c.is_ascii_alphanumeric())
        && !RESERVED.contains(&name)
}

/// Make `bindings` callable from every script on this thread.
///
/// `module` is the `renzora` object, which receives every binding; the global
/// object receives each one too, except a name that is already a global the
/// plugin did not put there (`Math`, `JSON`, `parseInt`, `console`, ...):
/// shadowing one of those would break it for every script at once, so such a
/// binding is reachable as `renzora.<name>` only, with a warning. `api_names`
/// are the plugin's own functions, which a binding may override, as a binding
/// registered after `register_api` overrides it in Lua. `installed` is what the
/// previous call put on the global object, and is replaced with what this one
/// did.
pub fn install(
    ctx: &mut Context,
    module: &JsObject,
    api_names: &[String],
    installed: &mut Vec<String>,
    bindings: &[Binding],
) -> JsResult<()> {
    let global = ctx.global_object();

    // Take out what the previous list installed, so a removed binding does not
    // linger as a function that does nothing.
    for name in std::mem::take(installed) {
        let _ = global.delete_property_or_throw(convert::key(&name), ctx);
        if !api_names.contains(&name) {
            let _ = module.delete_property_or_throw(convert::key(&name), ctx);
        }
    }

    for b in bindings {
        let binding = b.clone();
        // SAFETY: the closure captures a `Binding`, which is strings and
        // enums and holds no `JsValue`, `JsObject` or any other type the
        // garbage collector traces, which is the condition `from_closure`
        // documents.
        let native = unsafe { NativeFunction::from_closure(move |_, args, ctx| call(ctx, &binding, args)) };
        let func = FunctionObjectBuilder::new(ctx.realm(), native)
            .name(convert::key(&b.name))
            .length(b.params.iter().map(|p| p.kind.arity()).sum())
            .build();
        module.set(convert::key(&b.name), func.clone(), false, ctx)?;

        if !is_identifier(&b.name) {
            bevy::log::warn!(
                "[JavaScript] binding `{}` is not a valid JavaScript name; call it as `renzora[\"{}\"](...)`",
                b.name,
                b.name
            );
            continue;
        }
        let taken = global.has_own_property(convert::key(&b.name), ctx)?;
        if taken && !api_names.contains(&b.name) {
            bevy::log::warn!(
                "[JavaScript] binding `{0}` would hide JavaScript's own `{0}`; \
                 call it as `renzora.{0}(...)` instead",
                b.name
            );
            continue;
        }
        global.set(convert::key(&b.name), func, false, ctx)?;
        installed.push(b.name.clone());
    }
    Ok(())
}

fn call(ctx: &mut Context, b: &Binding, args: &[JsValue]) -> JsResult<JsValue> {
    match &b.kind {
        BindingKind::Action { action } => {
            let mut packed = Vec::with_capacity(b.params.len());
            let mut i = 0;
            for p in &b.params {
                let (value, used) = arg_value(ctx, args, i, p.kind);
                packed.push((p.name.clone(), value));
                i += used;
            }
            push_command(ScriptCommand::Action {
                name: action.clone(),
                target_entity: None,
                args: packed,
            });
            Ok(JsValue::undefined())
        }
        BindingKind::Read { component, field } => {
            // Placeholders are positional over the SCRIPT arguments, not the
            // packed ones, so a `Vec3` parameter does not shift the numbering.
            let subs: Vec<String> = (0..b.params.len().max(args.len()))
                .map(|i| args.get(i).map(|v| convert::display(v, ctx)).unwrap_or_default())
                .collect();
            let component = substitute(component, &subs);
            let field = substitute(field, &subs);
            match host::call_get(None, &component, &field) {
                Some(v) => convert::from_property_value(ctx, v),
                None => Ok(JsValue::null()),
            }
        }
        BindingKind::Translate => {
            let key = args.first().map(|v| convert::display(v, ctx)).unwrap_or_default();
            Ok(convert::text(&renzora::lang::t(&key)))
        }
        // A kind added to the contract after this plugin was written: say so
        // rather than doing nothing silently.
        #[allow(unreachable_patterns)]
        _ => Err(JsNativeError::typ()
            .with_message(format!("`{}` is a kind of script function this plugin does not know", b.name))
            .into()),
    }
}

/// One declared parameter from the script's arguments, and how many arguments
/// it consumed.
///
/// A missing or wrong-typed argument becomes the type's zero rather than an
/// error, matching the Lua plugin (and the hand-written bindings before it):
/// scripts depend on `apply_force(1, 2)` meaning a `z` of zero.
///
/// Two conveniences Lua cannot offer: a `Vec3` parameter takes one vector (a
/// `Vec3`, any `{x, y, z}` object or a 3-number array) as well as three
/// numbers, and a `Str` parameter given any other object or array receives its
/// JSON text, so `mysql_query_with(sql, [3], "scores")` works without a
/// `json_encode`.
fn arg_value(ctx: &mut Context, args: &[JsValue], i: usize, kind: ParamKind) -> (ActionValue, usize) {
    let v = args.get(i).filter(|v| !v.is_null_or_undefined());
    let f = |v: Option<&JsValue>| v.and_then(convert::number).unwrap_or(0.0) as f32;
    match kind {
        ParamKind::Float => (ActionValue::Float(f(v)), 1),
        ParamKind::Int => (ActionValue::Int(v.and_then(convert::integer).unwrap_or(0)), 1),
        ParamKind::Bool => (ActionValue::Bool(v.is_some_and(JsValue::to_boolean)), 1),
        ParamKind::Str => (
            ActionValue::String(match v {
                None => String::new(),
                Some(v) => match convert::string(v) {
                    Some(s) => s,
                    None if v.is_object() && !v.is_callable() => convert::to_json(v, ctx, 0)
                        .map(|j| j.to_string())
                        .unwrap_or_else(|_| convert::display(v, ctx)),
                    None => convert::display(v, ctx),
                },
            }),
            1,
        ),
        ParamKind::Vec3 => {
            let as_vector = match v {
                Some(v) if convert::number(v).is_none() => convert::shape(v, ctx),
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
