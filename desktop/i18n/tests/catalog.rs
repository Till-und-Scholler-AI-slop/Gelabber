//! Every catalog entry, in both languages (the native side of
//! web/src/i18n/catalogs.test.ts): well-formed keys, no empty text, the
//! same placeholders and tags in German and English, and every namespace
//! file listed in `NAMESPACES`.

use std::collections::BTreeSet;

use gelabber_i18n::NAMESPACES;

/// `{name}` placeholders.
fn placeholders(text: &str) -> BTreeSet<String> {
    let mut out = BTreeSet::new();
    let mut rest = text;
    while let Some(open) = rest.find('{') {
        let after = &rest[open + 1..];
        match after.find('}') {
            Some(end) => {
                out.insert(after[..end].to_owned());
                rest = &after[end + 1..];
            }
            None => break,
        }
    }
    out
}

/// The tags of `<link>…</link>` pieces.
fn tags(text: &str) -> BTreeSet<String> {
    gelabber_i18n::pieces(text)
        .into_iter()
        .filter_map(|p| p.tag.map(str::to_owned))
        .collect()
}

#[test]
fn every_entry_has_both_languages_alike() {
    let mut problems = Vec::new();
    for (ns, entries) in NAMESPACES {
        let mut keys = BTreeSet::new();
        for (key, de, en) in *entries {
            let at = format!("{ns}::{key}");
            let snake = key.starts_with(|c: char| c.is_ascii_lowercase())
                && key
                    .chars()
                    .all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == '_')
                && !key.contains("__")
                && !key.ends_with('_');
            if !snake {
                problems.push(format!("{at}: the key is not snake_case"));
            }
            if !keys.insert(*key) {
                problems.push(format!("{at}: the key is there twice"));
            }
            if de.trim().is_empty() || en.trim().is_empty() {
                problems.push(format!("{at}: a text is empty"));
            }
            if placeholders(de) != placeholders(en) {
                problems.push(format!("{at}: the placeholders differ"));
            }
            if tags(de) != tags(en) {
                problems.push(format!("{at}: the tags differ"));
            }
        }
    }
    assert!(problems.is_empty(), "{}", problems.join("\n"));
}

#[test]
fn every_namespace_file_is_listed() {
    let src = concat!(env!("CARGO_MANIFEST_DIR"), "/src");
    // The crate's own machinery, not catalogs.
    let infrastructure = ["lib", "intl", "system"];
    let mut files: Vec<String> = std::fs::read_dir(src)
        .unwrap()
        .filter_map(|e| {
            let name = e.unwrap().file_name().into_string().unwrap();
            name.strip_suffix(".rs").map(str::to_owned)
        })
        .filter(|name| !infrastructure.contains(&name.as_str()))
        .collect();
    files.sort();
    let mut listed: Vec<String> = NAMESPACES.iter().map(|(ns, _)| ns.to_string()).collect();
    listed.sort();
    assert_eq!(files, listed, "src/<namespace>.rs and NAMESPACES differ");
}

#[test]
fn the_helpers_find_placeholders_and_tags() {
    assert_eq!(
        placeholders("{count} neue {kind}"),
        BTreeSet::from(["count".to_owned(), "kind".to_owned()])
    );
    assert_eq!(
        tags("Mehr in der <link>Hilfe</link>."),
        BTreeSet::from(["link".to_owned()])
    );
}
