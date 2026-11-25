//! How the code editor colours `.js` and `.ts` files.
//!
//! The editor has no idea which languages a project will be scripted in, so
//! each language plugin describes its own. Registered from the plugin's
//! `build`, so the colouring arrives and leaves with the interpreter.

use renzora::CodeSyntax;

/// Keywords both languages share, plus the literal words that read like them.
macro_rules! js_keywords {
    ($($extra:literal),* $(,)?) => {
        &[
            "async", "await", "break", "case", "catch", "class", "const", "continue",
            "debugger", "default", "delete", "do", "else", "export", "extends", "finally",
            "for", "from", "function", "get", "if", "import", "in", "instanceof", "let",
            "new", "of", "return", "set", "static", "super", "switch", "this", "throw",
            "try", "typeof", "var", "void", "while", "with", "yield", "true", "false",
            "null", "undefined", "NaN", "Infinity",
            $($extra),*
        ]
    };
}

/// JavaScript: capitalised names (`Map`, `JSON`, a class) colour as types,
/// template literals may span lines, and `$` is an identifier character.
pub const JAVASCRIPT: CodeSyntax = CodeSyntax {
    keywords: js_keywords!(),
    caps_are_types: true,
    line_comment: Some("//"),
    block_comment: Some(("/*", "*/")),
    multiline_strings: &["`"],
    identifier_chars: "$",
    ..CodeSyntax::new("JavaScript", &["js", "mjs", "cjs"])
};

/// TypeScript: JavaScript plus the words of the type layer, and its primitive
/// types.
pub const TYPESCRIPT: CodeSyntax = CodeSyntax {
    name: "TypeScript",
    extensions: &["ts", "mts", "cts"],
    keywords: js_keywords!(
        "abstract", "as", "asserts", "declare", "enum", "implements", "infer",
        "interface", "is", "keyof", "module", "namespace", "override", "private",
        "protected", "public", "readonly", "satisfies", "type", "unique",
    ),
    types: &[
        "any", "bigint", "boolean", "never", "number", "object", "string", "symbol",
        "unknown",
    ],
    ..JAVASCRIPT
};
