//! How the code editor colours `.cs` files.
//!
//! The editor has no idea which languages a project will be scripted in, so
//! each language plugin describes its own. Registered from the plugin's
//! `build`, so the colouring arrives and leaves with the language.

use renzora::CodeSyntax;

/// C#: capitalised names (`Vector3`, `Script`, a class) colour as types, which
/// is the language's own naming convention. `"""` raw string literals may span
/// lines. A verbatim `@"..."` string may too, but closes on a bare `"`, which
/// the editor's tokenizer (a string closes on the delimiter that opened it)
/// cannot express; it colours correctly on one line.
pub const CSHARP: CodeSyntax = CodeSyntax {
    keywords: &[
        "abstract", "as", "base", "break", "case", "catch", "checked", "class", "const",
        "continue", "default", "delegate", "do", "else", "enum", "event", "explicit", "extern",
        "false", "finally", "fixed", "for", "foreach", "goto", "if", "implicit", "in",
        "interface", "internal", "is", "lock", "namespace", "new", "null", "operator", "out",
        "override", "params", "private", "protected", "public", "readonly", "ref", "return",
        "sealed", "sizeof", "stackalloc", "static", "struct", "switch", "this", "throw", "true",
        "try", "typeof", "unchecked", "unsafe", "using", "virtual", "volatile", "while",
        // Contextual keywords a script meets in practice.
        "async", "await", "file", "get", "set", "init", "nameof", "partial", "record", "required",
        "value", "when", "where", "with", "yield", "global", "and", "or", "not",
    ],
    types: &[
        "bool", "byte", "char", "decimal", "double", "dynamic", "float", "int", "long", "nint",
        "nuint", "object", "sbyte", "short", "string", "uint", "ulong", "ushort", "var", "void",
    ],
    caps_are_types: true,
    line_comment: Some("//"),
    block_comment: Some(("/*", "*/")),
    quotes: "\"'",
    multiline_strings: &["\"\"\""],
    ..CodeSyntax::new("C#", &["cs", "csx"])
};
