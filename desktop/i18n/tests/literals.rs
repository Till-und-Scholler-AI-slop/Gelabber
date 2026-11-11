//! The hardcoded-string check (the native side of the web's
//! `gelabber/no-literal-ui-text`): text people read comes from the
//! catalogs in src/, not from string literals in the app's code.
//!
//! It scans desktop/ui/src, desktop/call/src, desktop/client/src and
//! desktop/gelabber/src and flags a string literal that contains an umlaut,
//! ß or "…", that contains two or more words, or that starts with a capital
//! letter followed by more words. It leaves out:
//!
//! - `#[cfg(test)]` items (the tests at a file's end), files named
//!   `tests*.rs` and `testkit.rs`;
//! - the messages of tracing macros, of `panic!`, `assert!` and their kin,
//!   and of `.expect(…)`: they are for developers;
//! - attributes, except thiserror's `#[error("…")]`;
//! - a literal on a line marked `// i18n-ok`, for intentional text such as
//!   "Gelabber".
//!
//! Files listed in `desktop/i18n/pending/<batch>.txt` are not checked yet:
//! each extraction batch moves its files' text into the catalogs and
//! deletes its list.

use std::collections::BTreeSet;
use std::path::{Path, PathBuf};

/// The directories checked, relative to desktop/.
const ROOTS: [&str; 4] = ["ui/src", "call/src", "client/src", "gelabber/src"];

/// Macros whose literals are not interface text.
const QUIET_MACROS: &[&str] = &[
    // tracing
    "trace",
    "debug",
    "info",
    "warn",
    "error",
    "event",
    "span",
    "trace_span",
    "debug_span",
    "info_span",
    "warn_span",
    "error_span",
    // panics and checks
    "panic",
    "assert",
    "assert_eq",
    "assert_ne",
    "debug_assert",
    "debug_assert_eq",
    "debug_assert_ne",
    "unreachable",
    "todo",
    "unimplemented",
    // files and the build
    "include_bytes",
    "include_str",
    "env",
    "option_env",
];

fn desktop() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .unwrap()
        .to_owned()
}

/// A string literal: the line it starts on and its text.
#[derive(Debug, PartialEq)]
struct Literal {
    line: usize,
    text: String,
}

#[derive(Clone, Debug, PartialEq)]
enum Token {
    None,
    Ident(String),
    Punct(char),
    Literal,
}

struct Frame {
    close: char,
    /// Its literals are not interface text.
    quiet: bool,
    /// An attribute: where its `[` is, and whether its first word came.
    attribute: Option<(usize, bool)>,
}

/// The string literals of `source` that may be interface text (see the
/// module's comment for what is left out).
fn literals(source: &str) -> Vec<Literal> {
    let chars: Vec<(usize, char)> = source.char_indices().collect();
    let at = |i: usize| chars.get(i).map(|&(_, c)| c);
    let next_significant = |mut i: usize| {
        while let Some(c) = at(i) {
            if !c.is_whitespace() {
                return Some(c);
            }
            i += 1;
        }
        None
    };
    let mut out = Vec::new();
    let mut frames: Vec<Frame> = Vec::new();
    let (mut prev, mut prev2) = (Token::None, Token::None);
    let mut quiet_macro = None::<bool>;
    let mut attribute_next = false;
    // After `#[cfg(test)]`: the depth of the item it marks, until the item
    // ends (its `;`, or the `}` of its body).
    let mut test_item: Option<usize> = None;
    let mut line = 1;
    let mut i = 0;
    while let Some(c) = at(i) {
        let quiet = test_item.is_some() || frames.iter().any(|f| f.quiet);
        // Comments.
        if c == '/' && at(i + 1) == Some('/') {
            while at(i).is_some_and(|c| c != '\n') {
                i += 1;
            }
            continue;
        }
        if c == '/' && at(i + 1) == Some('*') {
            let mut depth = 0;
            while let Some(c) = at(i) {
                if c == '/' && at(i + 1) == Some('*') {
                    depth += 1;
                    i += 2;
                } else if c == '*' && at(i + 1) == Some('/') {
                    depth -= 1;
                    i += 2;
                    if depth == 0 {
                        break;
                    }
                } else {
                    line += usize::from(c == '\n');
                    i += 1;
                }
            }
            continue;
        }
        if c.is_whitespace() {
            line += usize::from(c == '\n');
            i += 1;
            continue;
        }
        // Strings.
        if c == '"' {
            let start = line;
            let (text, end, lines) = quoted(&chars, i + 1);
            if !quiet {
                out.push(Literal { line: start, text });
            }
            line += lines;
            i = end;
            (prev2, prev) = (prev, Token::Literal);
            continue;
        }
        // Char literals and lifetimes.
        if c == '\'' {
            if at(i + 1) == Some('\\') {
                i += 2;
                while at(i).is_some_and(|c| c != '\'') {
                    i += 1;
                }
                i += 1;
            } else if at(i + 2) == Some('\'') {
                i += 3;
            } else {
                i += 1;
                while at(i).is_some_and(|c| c.is_alphanumeric() || c == '_') {
                    i += 1;
                }
            }
            (prev2, prev) = (prev, Token::Literal);
            continue;
        }
        if c.is_alphabetic() || c == '_' {
            let from = i;
            while at(i).is_some_and(|c| c.is_alphanumeric() || c == '_') {
                i += 1;
            }
            let word: String = chars[from..i].iter().map(|&(_, c)| c).collect();
            // Prefixed strings: r"…", r#"…"#, b"…", br"…", c"…", b'…'.
            if matches!(word.as_str(), "r" | "br" | "cr") && matches!(at(i), Some('"' | '#')) {
                let mut hashes = 0;
                while at(i + hashes) == Some('#') {
                    hashes += 1;
                }
                if at(i + hashes) == Some('"') {
                    let start = line;
                    let mut j = i + hashes + 1;
                    let body = j;
                    loop {
                        match at(j) {
                            None => break,
                            Some('"') if (1..=hashes).all(|h| at(j + h) == Some('#')) => break,
                            Some(c) => {
                                line += usize::from(c == '\n');
                                j += 1;
                            }
                        }
                    }
                    if word == "r" && !quiet {
                        let text = chars[body..j].iter().map(|&(_, c)| c).collect();
                        out.push(Literal { line: start, text });
                    }
                    i = j + 1 + hashes;
                    (prev2, prev) = (prev, Token::Literal);
                    continue;
                }
                // A raw identifier (r#type): the word after the hash.
                i += hashes;
                continue;
            }
            if matches!(word.as_str(), "b" | "c") && at(i) == Some('"') {
                let (_, end, lines) = quoted(&chars, i + 1);
                line += lines;
                i = end;
                (prev2, prev) = (prev, Token::Literal);
                continue;
            }
            if word == "b" && at(i) == Some('\'') {
                continue;
            }
            if let Some(frame) = frames.last_mut()
                && let Some((_, first)) = &mut frame.attribute
                && !*first
            {
                *first = true;
                // thiserror's messages are shown; other attributes are not text.
                frame.quiet = word != "error";
            }
            (prev2, prev) = (prev, Token::Ident(word));
            continue;
        }
        match c {
            '!' => {
                if let Token::Ident(name) = &prev
                    && matches!(next_significant(i + 1), Some('(' | '[' | '{'))
                {
                    quiet_macro = Some(QUIET_MACROS.contains(&name.as_str()));
                }
            }
            '#' => attribute_next = matches!(next_significant(i + 1), Some('[' | '!')),
            '(' | '[' | '{' => {
                let close = match c {
                    '(' => ')',
                    '[' => ']',
                    _ => '}',
                };
                let attribute = (c == '[' && attribute_next).then_some((i, false));
                let expect =
                    c == '(' && prev == Token::Ident("expect".into()) && prev2 == Token::Punct('.');
                let mut quiet = quiet_macro.take() == Some(true) || expect || attribute.is_some();
                if c == '{' && test_item == Some(frames.len()) {
                    test_item = None;
                    quiet = true;
                }
                frames.push(Frame {
                    close,
                    quiet,
                    attribute,
                });
                attribute_next = false;
            }
            ')' | ']' | '}' => {
                if let Some(frame) = frames.pop_if(|f| f.close == c) {
                    // The marked item was a field or variant; its list ended.
                    if test_item.is_some_and(|depth| frames.len() < depth) {
                        test_item = None;
                    }
                    if let Some((open, _)) = frame.attribute {
                        let inner: String = chars[open + 1..i]
                            .iter()
                            .map(|&(_, c)| c)
                            .filter(|c| !c.is_whitespace())
                            .collect();
                        if inner == "cfg(test)" || inner.starts_with("cfg(all(test") {
                            test_item = Some(frames.len());
                        }
                    }
                }
            }
            ';' | ',' if test_item == Some(frames.len()) => test_item = None,
            _ => {}
        }
        if c != '!' {
            quiet_macro = None;
        }
        (prev2, prev) = (prev, Token::Punct(c));
        i += 1;
    }
    out
}

/// A `"…"` string from just after its opening quote: its text with the
/// common escapes read, the index after the closing quote, and the line
/// breaks inside.
fn quoted(chars: &[(usize, char)], mut i: usize) -> (String, usize, usize) {
    let mut text = String::new();
    let mut lines = 0;
    while let Some(&(_, c)) = chars.get(i) {
        match c {
            '"' => return (text, i + 1, lines),
            '\\' => {
                let escaped = chars.get(i + 1).map_or(' ', |&(_, c)| c);
                match escaped {
                    'n' | 't' | 'r' => text.push(' '),
                    '\n' => {
                        lines += 1;
                        text.push(' ');
                    }
                    other => text.push(other),
                }
                i += 2;
            }
            c => {
                lines += usize::from(c == '\n');
                text.push(c);
                i += 1;
            }
        }
    }
    (text, i, lines)
}

/// Text that reads like words for people.
fn looks_like_text(text: &str) -> bool {
    if text.contains(['ä', 'ö', 'ü', 'Ä', 'Ö', 'Ü', 'ß', '…']) {
        return true;
    }
    let tokens: Vec<&str> = text.split_whitespace().collect();
    let words = tokens
        .iter()
        .filter(|t| t.starts_with(char::is_alphabetic))
        .count();
    // "Gelabber: {count}": a capital word, then more (a path is not one).
    let capital = text.starts_with(char::is_uppercase)
        && tokens
            .iter()
            .skip(1)
            .any(|t| t.starts_with(|c: char| c.is_alphabetic() || c == '{'));
    words >= 2 || capital
}

/// The pending lists' files, relative to desktop/.
fn pending() -> BTreeSet<String> {
    let dir = desktop().join("i18n/pending");
    let Ok(entries) = std::fs::read_dir(&dir) else {
        return BTreeSet::new();
    };
    let mut out = BTreeSet::new();
    for entry in entries {
        let path = entry.unwrap().path();
        if path.extension().is_none_or(|e| e != "txt") {
            continue;
        }
        for line in std::fs::read_to_string(&path).unwrap().lines() {
            let line = line.trim();
            if !line.is_empty() && !line.starts_with('#') {
                out.insert(line.to_owned());
            }
        }
    }
    out
}

fn rust_files(dir: &Path, out: &mut Vec<PathBuf>) {
    for entry in std::fs::read_dir(dir).unwrap() {
        let path = entry.unwrap().path();
        if path.is_dir() {
            rust_files(&path, out);
        } else if path.extension().is_some_and(|e| e == "rs") {
            out.push(path);
        }
    }
}

fn checked(name: &str) -> bool {
    !name.starts_with("tests") && name != "testkit.rs"
}

#[test]
fn interface_text_comes_from_the_catalogs() {
    let desktop = desktop();
    let pending = pending();
    let mut files = Vec::new();
    for root in ROOTS {
        rust_files(&desktop.join(root), &mut files);
    }
    files.sort();
    let mut found = Vec::new();
    for path in files {
        let rel = path
            .strip_prefix(&desktop)
            .unwrap()
            .to_string_lossy()
            .replace('\\', "/");
        let name = path.file_name().unwrap().to_string_lossy();
        if !checked(&name) || pending.contains(&rel) {
            continue;
        }
        let source = std::fs::read_to_string(&path).unwrap();
        let lines: Vec<&str> = source.lines().collect();
        for literal in literals(&source) {
            let marked = lines
                .get(literal.line - 1)
                .is_some_and(|l| l.contains("// i18n-ok"));
            if !marked && looks_like_text(&literal.text) {
                found.push(format!("{rel}:{}: {:?}", literal.line, literal.text));
            }
        }
    }
    assert!(
        found.is_empty(),
        "Text outside the catalogs (desktop/i18n/src; mark an intentional \
         literal with `// i18n-ok`):\n{}",
        found.join("\n")
    );
}

/// A pending list names files that exist, inside the checked directories.
#[test]
fn the_pending_lists_name_checked_files() {
    let desktop = desktop();
    let wrong: Vec<String> = pending()
        .into_iter()
        .filter(|rel| {
            !desktop.join(rel).is_file() || !ROOTS.iter().any(|r| rel.starts_with(&format!("{r}/")))
        })
        .collect();
    assert!(wrong.is_empty(), "not a checked file: {wrong:?}");
}

#[test]
fn the_scanner_finds_text_and_leaves_the_rest() {
    let source = r##"
//! "Kein Text" in a comment.
use x as y; /* "auch nicht" */
const NAME: &str = "Verlassen";
fn label() -> &'static str {
    let c = '"';
    let quiet = "Gelabber öffnen"; // i18n-ok
    tracing::warn!("server did not answer: {}", 5);
    panic!("this should not happen");
    let x = y.expect("a client for the test");
    let r = r#"Ein "roher" Text"#;
    let b = b"two words";
    "Mit dem Server verbinden"
}
#[derive(Debug, thiserror::Error)]
enum E {
    #[error("Die Verbindung ist weg")]
    Gone,
    #[serde(rename = "two words")]
    Renamed,
}
#[cfg(test)]
mod tests {
    #[test]
    fn t() {
        let _ = "Text in einem Test";
    }
}
fn after() -> &'static str {
    "Nach dem Test"
}
"##;
    let texts: Vec<(usize, String)> = literals(source)
        .into_iter()
        .map(|l| (l.line, l.text))
        .collect();
    assert_eq!(
        texts,
        [
            (4, "Verlassen".to_owned()),
            (7, "Gelabber öffnen".to_owned()),
            (11, "Ein \"roher\" Text".to_owned()),
            (13, "Mit dem Server verbinden".to_owned()),
            (17, "Die Verbindung ist weg".to_owned()),
            (30, "Nach dem Test".to_owned()),
        ]
    );
    assert!(looks_like_text("Gelabber öffnen"));
    assert!(looks_like_text("Wird geladen…"));
    assert!(looks_like_text("Mit dem Server verbinden"));
    assert!(looks_like_text("{name} ist live"));
    assert!(looks_like_text("Gelabber: {count}"));
    assert!(!looks_like_text("Verlassen"));
    assert!(!looks_like_text("gelabber-mode"));
    assert!(!looks_like_text("GET /me/themes"));
    assert!(!looks_like_text("{} {}"));
    assert!(!looks_like_text("application/json"));
}
