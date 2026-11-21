// SPDX-License-Identifier: AGPL-3.0-or-later
//! Coherence des traductions : toutes les langues ont les memes cles et les memes variables, aucune cle utilisee absente, aucune cle morte.

use std::collections::{BTreeMap, BTreeSet};
use std::fs;
use std::path::Path;

/// Langues presentes : un dossier `locales/<code>/main.ftl` par langue.
fn languages() -> Vec<String> {
    let dir = Path::new(env!("CARGO_MANIFEST_DIR")).join("locales");
    let mut found: Vec<String> = fs::read_dir(dir)
        .unwrap()
        .flatten()
        .filter(|e| e.path().join("main.ftl").is_file())
        .map(|e| e.file_name().to_string_lossy().into_owned())
        .collect();
    found.sort();
    found
}

/// Messages d'une langue : cle -> valeur, lignes de continuation et attributs inclus (meme lecture que `tools/i18n.py`).
fn messages(lang: &str) -> BTreeMap<String, String> {
    let path = Path::new(env!("CARGO_MANIFEST_DIR")).join(format!("locales/{lang}/main.ftl"));
    let mut out = BTreeMap::new();
    let mut key: Option<String> = None;
    for line in fs::read_to_string(path).unwrap().lines() {
        if line.starts_with('#') {
            continue;
        }
        let named = line
            .split_once('=')
            .filter(|(k, _)| !k.starts_with([' ', '\t']) && is_key(k.trim()));
        if let Some((k, v)) = named {
            out.insert(k.trim().to_string(), v.trim().to_string());
            key = Some(k.trim().to_string());
        } else if line.starts_with([' ', '\t']) {
            if let Some(m) = key.as_ref().and_then(|k| out.get_mut(k)) {
                m.push('\n');
                m.push_str(line.trim());
            }
        } else if !line.trim().is_empty() {
            key = None;
        }
    }
    out
}

fn is_key(k: &str) -> bool {
    k.chars().next().is_some_and(|c| c.is_ascii_alphabetic())
        && k.chars()
            .all(|c| c.is_ascii_alphanumeric() || c == '-' || c == '_')
}

fn keys(lang: &str) -> BTreeSet<String> {
    messages(lang).into_keys().collect()
}

/// Variables `$nom` d'un message.
fn variables(text: &str) -> BTreeSet<String> {
    let mut out = BTreeSet::new();
    let mut rest = text;
    while let Some(i) = rest.find('$') {
        rest = &rest[i + 1..];
        let name: String = rest
            .chars()
            .take_while(|c| c.is_ascii_alphanumeric() || *c == '-' || *c == '_')
            .collect();
        if !name.is_empty() {
            out.insert(name);
        }
    }
    out
}

/// Cles passees a `tr!(...)` / `move_tr!(...)` dans les sources.
fn used_keys() -> BTreeSet<String> {
    let dir = Path::new(env!("CARGO_MANIFEST_DIR")).join("src");
    let mut used = BTreeSet::new();
    for entry in fs::read_dir(dir).unwrap() {
        let path = entry.unwrap().path();
        if path.extension().is_none_or(|e| e != "rs") {
            continue;
        }
        let text = fs::read_to_string(&path).unwrap();
        for marker in ["tr!(\"", "move_tr!(\"", "tk!(\""] {
            let mut rest = text.as_str();
            let mut before = String::new();
            while let Some(i) = rest.find(marker) {
                // `include_str!("...")` se termine lui aussi par `tr!("` : on exige une frontiere de mot.
                before.push_str(&rest[..i]);
                let boundary = !before
                    .chars()
                    .last()
                    .is_some_and(|c| c.is_ascii_alphanumeric());
                rest = &rest[i + marker.len()..];
                before.push_str(marker);
                if boundary && let Some(end) = rest.find('"') {
                    used.insert(rest[..end].to_string());
                }
            }
        }
    }
    used
}

#[test]
fn french_and_english_are_always_there() {
    let found = languages();
    assert!(
        found.contains(&"fr".to_string()) && found.contains(&"en".to_string()),
        "{found:?}"
    );
}

/// Toute langue ajoutee (`make i18n-new`) doit definir exactement les cles de l'anglais, avec les memes variables : sinon un
/// message manquerait, ou une variable resterait sans valeur, a l'affichage.
#[test]
fn every_language_defines_the_same_keys_and_variables_as_english() {
    let reference = messages("en");
    for lang in languages().into_iter().filter(|l| l != "en") {
        let own = messages(&lang);
        let missing: Vec<_> = reference.keys().filter(|k| !own.contains_key(*k)).collect();
        let extra: Vec<_> = own.keys().filter(|k| !reference.contains_key(*k)).collect();
        assert!(missing.is_empty(), "{lang} : cles absentes : {missing:?}");
        assert!(extra.is_empty(), "{lang} : cles en trop : {extra:?}");
        let bad: Vec<_> = own
            .iter()
            .filter(|(k, v)| variables(v) != variables(&reference[*k]))
            .map(|(k, _)| k)
            .collect();
        assert!(
            bad.is_empty(),
            "{lang} : variables differentes de l'anglais : {bad:?}"
        );
    }
}

#[test]
fn every_key_used_in_the_code_is_defined() {
    let (defined, used) = (keys("en"), used_keys());
    assert!(
        !used.is_empty(),
        "le balayage des sources ne trouve aucune cle"
    );
    let missing: Vec<_> = used.difference(&defined).collect();
    assert!(
        missing.is_empty(),
        "cles utilisees mais non definies : {missing:?}"
    );
}

#[test]
fn no_defined_key_is_unused() {
    let (defined, used) = (keys("en"), used_keys());
    let dead: Vec<_> = defined.difference(&used).collect();
    assert!(dead.is_empty(), "cles definies mais inutilisees : {dead:?}");
}
