//! Turning a `.js` or `.ts` file into the text Boa runs.
//!
//! Two jobs, both done with oxc (which Boa does not need for anything else):
//!
//! * **TypeScript** is parsed, its types stripped (and `enum` / `namespace`
//!   lowered to the objects they mean at run time), and printed back as plain
//!   JavaScript. Nothing is type-checked: the code runs exactly as `tsc
//!   --noCheck` would emit it, and the checking is left to the user's editor,
//!   against `renzora.d.ts`.
//! * **`export`** is removed from both languages. A script is not a module (see
//!   [`crate::runtime`] for how it is run), but `export function on_update()`
//!   is what a TypeScript user writes, because a file with no `export` is a
//!   *global* script to the TypeScript language service, and two scripts that
//!   both declare `const speed` then collide in the editor though they never
//!   meet at run time. So `export` is accepted and means nothing, and
//!   `export {}` (the usual way to say "this file is a module") is allowed too.
//!
//! The export keywords are blanked with spaces in the ORIGINAL text rather than
//! dropped from the printed output, which keeps every line and column of a
//! plain `.js` file where its author wrote it: an error at `player.js:12:5` is
//! at line 12. TypeScript is reprinted, so its lines move; the printer's source
//! map gives them back, as [`Prepared::lines`].
//!
//! `import` is refused with a message. There is nothing to import from: every
//! engine function is a global, and scripts do not load other files.

use std::path::Path;

use oxc_allocator::Allocator;
use oxc_ast::ast::{ExportDefaultDeclarationKind, Statement};
use oxc_codegen::{Codegen, CodegenOptions};
use oxc_parser::Parser;
use oxc_semantic::SemanticBuilder;
use oxc_span::{GetSpan, SourceType, Span};
use oxc_transformer::{TransformOptions, Transformer};

/// `SyntaxError: message (name:line:col)` for the first error oxc reported,
/// worded like Boa's own so the two read alike in the console.
///
/// A macro only so the diagnostic type need not be named: it lives in
/// `oxc_diagnostics`, which would otherwise be one more dependency line for one
/// type name.
macro_rules! first_error {
    ($name:expr, $source:expr, $errors:expr) => {{
        let (name, source): (&str, &str) = ($name, $source);
        match $errors.next() {
            None => format!("SyntaxError ({name})"),
            Some(e) => {
                let at = e
                    .labels
                    .first()
                    .map(|l| format!(":{}", line_col(source, l.offset() as u32)))
                    .unwrap_or_default();
                format!("SyntaxError: {} ({name}{at})", e.message)
            }
        }
    }};
}

/// A script ready for the interpreter.
pub struct Prepared {
    pub code: String,
    /// For TypeScript, the original (0-based) line of each printed line, so an
    /// error can name the line the author wrote. `None` for JavaScript, whose
    /// lines never move.
    pub lines: Option<Vec<u32>>,
}

pub fn is_typescript(path: &str) -> bool {
    Path::new(path)
        .extension()
        .and_then(|e| e.to_str())
        .is_some_and(|e| e.eq_ignore_ascii_case("ts"))
}

/// Prepare `source` (the file at `path`) for Boa.
///
/// `name` is the file name errors are reported under. `Err` is a syntax error
/// (or a refused `import`), worded like Boa's own errors: `SyntaxError: what
/// (name:line:col)`.
pub fn prepare(path: &str, name: &str, source: &str) -> Result<Prepared, String> {
    let ts = is_typescript(path);
    // Most plain scripts never mention either keyword, and they skip oxc
    // entirely: Boa parses them as they are.
    let mentions_modules = source.contains("export") || source.contains("import");
    if !ts && !mentions_modules {
        return Ok(Prepared {
            code: source.to_string(),
            lines: None,
        });
    }
    let source = if mentions_modules {
        strip_exports(name, source, ts)?
    } else {
        source.to_string()
    };
    if !ts {
        return Ok(Prepared {
            code: source,
            lines: None,
        });
    }
    strip_types(name, &source)
}

/// Blank every `export` keyword (and `export {}` / `export { a, b }` lists) out
/// of `source`, keeping its length and its line breaks.
///
/// A JavaScript file that does not parse as a module is returned unchanged
/// (with its `export`s, if it really had any), so the syntax error Boa reports
/// is the one the author sees, with Boa's wording.
fn strip_exports(name: &str, source: &str, ts: bool) -> Result<String, String> {
    let allocator = Allocator::default();
    let source_type = if ts { SourceType::ts() } else { SourceType::mjs() };
    let parsed = Parser::new(&allocator, source, source_type).parse();
    if parsed.diagnostics.has_errors() {
        if ts {
            // TypeScript has no second parser to defer to.
            return Err(first_error!(name, source, parsed.diagnostics.errors()));
        }
        return Ok(source.to_string());
    }

    let mut blank: Vec<Span> = Vec::new();
    for stmt in &parsed.program.body {
        match stmt {
            Statement::ExportNamedDeclaration(d) => match (&d.declaration, &d.source) {
                // `export function f() {}`, `export const x = 1`,
                // `export interface P {}`: keep the declaration.
                (Some(decl), _) => blank.push(Span::new(d.span.start, decl.span().start)),
                // `export { a, b }` and `export {}`: the names are already in
                // scope, so the list says nothing at run time.
                (None, None) => blank.push(d.span),
                // `export { a } from "./other"` is an import in disguise.
                (None, Some(_)) => return Err(refused(name, source, d.span, "export ... from")),
            },
            Statement::ExportDefaultDeclaration(d) => match &d.declaration {
                ExportDefaultDeclarationKind::FunctionDeclaration(f) if f.id.is_some() => {
                    blank.push(Span::new(d.span.start, f.span.start))
                }
                ExportDefaultDeclarationKind::ClassDeclaration(c) if c.id.is_some() => {
                    blank.push(Span::new(d.span.start, c.span.start))
                }
                ExportDefaultDeclarationKind::TSInterfaceDeclaration(i) => {
                    blank.push(Span::new(d.span.start, i.span.start))
                }
                // `export default 5` or an anonymous function: with the
                // keywords gone it would be an expression nobody can name.
                _ => {
                    return Err(format!(
                        "SyntaxError: `export default` needs a named function or class here; \
                         nothing imports a script, so a plain declaration does the same ({name}:{})",
                        line_col(source, d.span.start)
                    ))
                }
            },
            Statement::ImportDeclaration(d) if d.import_kind.is_type() => blank.push(d.span),
            Statement::ImportDeclaration(d) => return Err(refused(name, source, d.span, "import")),
            Statement::ExportAllDeclaration(d) => return Err(refused(name, source, d.span, "export * from")),
            Statement::TSExportAssignment(d) => return Err(refused(name, source, d.span, "export =")),
            Statement::TSNamespaceExportDeclaration(d) => blank.push(d.span),
            Statement::TSImportEqualsDeclaration(d) if !d.import_kind.is_type() => {
                return Err(refused(name, source, d.span, "import ... = require"))
            }
            _ => {}
        }
    }
    let mut bytes = source.as_bytes().to_vec();
    for span in blank {
        for b in &mut bytes[span.start as usize..span.end as usize] {
            // Line breaks stay, so every later line keeps its number. Every
            // other byte becomes a space, which is safe in the middle of a
            // multi-byte character only because the spans begin and end on
            // token boundaries, and no token boundary is inside a character.
            if *b != b'\n' && *b != b'\r' {
                *b = b' ';
            }
        }
    }
    String::from_utf8(bytes).map_err(|e| format!("{name}: {e}"))
}

/// Strip TypeScript's types and print plain JavaScript, with the line map.
fn strip_types(name: &str, source: &str) -> Result<Prepared, String> {
    let allocator = Allocator::default();
    // A script, not a module: the exports are gone by now, and module mode
    // would also make the whole file strict, which a `.js` script is not.
    let source_type = SourceType::ts().with_script(true);
    let parsed = Parser::new(&allocator, source, source_type).parse();
    if parsed.diagnostics.has_errors() {
        return Err(first_error!(name, source, parsed.diagnostics.errors()));
    }
    let mut program = parsed.program;
    let scoping = SemanticBuilder::new().build(&program).semantic.into_scoping();
    // The defaults: strip types, lower `enum` / `namespace` / parameter
    // properties, and leave every JavaScript feature as written. Boa runs
    // ES2024-era code natively, so lowering classes or `?.` would only make
    // the output slower and its errors harder to read.
    let options = TransformOptions::default();
    let transformed = Transformer::new(&allocator, Path::new(name), &options).build_with_scoping(scoping, &mut program);
    if transformed.diagnostics.has_errors() {
        return Err(first_error!(name, source, transformed.diagnostics.errors()));
    }
    let printed = Codegen::new()
        .with_options(CodegenOptions {
            source_map_path: Some(name.into()),
            ..CodegenOptions::default()
        })
        .build(&program);

    // Each printed line, mapped to the first original line any token on it
    // came from. A printed line with no token of its own (a closing brace the
    // printer put on a line by itself) inherits the line above, which is the
    // statement it closes.
    let mut lines: Vec<u32> = Vec::new();
    if let Some(map) = &printed.map {
        for token in map.get_tokens() {
            let dst = token.get_dst_line() as usize;
            if lines.len() <= dst {
                lines.resize(dst + 1, u32::MAX);
            }
            if lines[dst] == u32::MAX {
                lines[dst] = token.get_src_line();
            }
        }
    }
    let mut last = 0;
    for line in &mut lines {
        if *line == u32::MAX {
            *line = last;
        }
        last = *line;
    }
    Ok(Prepared {
        code: printed.code,
        lines: Some(lines),
    })
}

/// A syntax error in `source` worded with its real position, for when Boa
/// refused the script.
///
/// Boa reports a parse error as "at line 4, col 1 (native)", counting from the
/// wrapper the runtime puts around a script, and an unclosed bracket is
/// reported at the wrapper's own last line, which the author never wrote. oxc
/// points at the file. `None` when oxc finds nothing wrong (the two parsers
/// disagree on some corner), and the caller falls back to Boa's text.
pub fn syntax_error(name: &str, source: &str) -> Option<String> {
    let allocator = Allocator::default();
    let parsed = Parser::new(&allocator, source, SourceType::script()).parse();
    if !parsed.diagnostics.has_errors() {
        return None;
    }
    let text = first_error!(name, source, parsed.diagnostics.errors());
    Some(text)
}

fn refused(name: &str, source: &str, span: Span, what: &str) -> String {
    format!(
        "SyntaxError: `{what}` is not supported: a script is not a module, and every engine \
         function is already a global ({name}:{})",
        line_col(source, span.start)
    )
}

/// The 1-based `line:col` of a byte offset, counting columns in characters.
fn line_col(source: &str, offset: u32) -> String {
    let offset = (offset as usize).min(source.len());
    let before = source.get(..offset).unwrap_or(source);
    let line = before.matches('\n').count() + 1;
    let col = before.rsplit('\n').next().map(|l| l.chars().count()).unwrap_or(0) + 1;
    format!("{line}:{col}")
}

/// Rewrite `name:LINE:COL` positions in an error from the printed TypeScript
/// to the original file's `name:LINE`, through the line map.
///
/// The column is dropped rather than guessed: the printer reflows every line,
/// so a column into printed text points nowhere in the author's.
pub fn map_positions(text: &str, name: &str, lines: &[u32]) -> String {
    let needle = format!("{name}:");
    let mut out = String::with_capacity(text.len());
    let mut rest = text;
    while let Some(at) = rest.find(&needle) {
        out.push_str(&rest[..at + needle.len()]);
        rest = &rest[at + needle.len()..];
        let digits = rest.bytes().take_while(u8::is_ascii_digit).count();
        let Ok(line) = rest[..digits].parse::<usize>() else {
            continue;
        };
        let original = line
            .checked_sub(1)
            .and_then(|i| lines.get(i))
            .map(|l| *l as usize + 1)
            .unwrap_or(line);
        out.push_str(&original.to_string());
        rest = &rest[digits..];
        // Skip the column that followed, if any.
        if let Some(after) = rest.strip_prefix(':') {
            let col = after.bytes().take_while(u8::is_ascii_digit).count();
            if col > 0 {
                rest = &after[col..];
            }
        }
    }
    out.push_str(rest);
    out
}
