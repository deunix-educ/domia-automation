// SPDX-License-Identifier: AGPL-3.0-or-later
//! Coverage guard (R11, `ROADMAP-mcp.md` section 7): every `#[server]` function of the interface must be classified in
//! `tests/mcp/coverage.tsv` (covered by a tool, excluded with a reason, or still to decide). A function added without a decision
//! fails `make ci`: the assistant surface cannot silently drift away from the interface.

use std::collections::{BTreeMap, BTreeSet};
use std::fs;
use std::path::{Path, PathBuf};

fn root() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("../..")
}

/// `(file, function)` of every `#[server(..)] pub async fn name` under `crates/dominet-web/src`.
fn server_functions() -> BTreeSet<(String, String)> {
    fn walk(dir: &Path, out: &mut Vec<PathBuf>) {
        for entry in fs::read_dir(dir).unwrap().flatten() {
            let path = entry.path();
            if path.is_dir() {
                walk(&path, out);
            } else if path.extension().is_some_and(|e| e == "rs") {
                out.push(path);
            }
        }
    }
    let mut files = vec![];
    walk(&root().join("crates/dominet-web/src"), &mut files);
    let mut found = BTreeSet::new();
    for file in files {
        let text = fs::read_to_string(&file).unwrap();
        let mut pending = false;
        for line in text.lines() {
            let t = line.trim_start();
            if t.starts_with("#[server") {
                pending = true;
            } else if pending && let Some(rest) = t.strip_prefix("pub async fn ") {
                let name: String = rest
                    .chars()
                    .take_while(|c| c.is_alphanumeric() || *c == '_')
                    .collect();
                let file_name = file.file_name().unwrap().to_string_lossy().into_owned();
                found.insert((file_name, name));
                pending = false;
            }
        }
    }
    found
}

struct Row {
    guard: String,
    tool: String,
    scope: String,
    status: String,
    note: String,
}

fn table() -> BTreeMap<(String, String), Row> {
    let text = fs::read_to_string(root().join("tests/mcp/coverage.tsv")).unwrap();
    let mut rows = BTreeMap::new();
    let mut lines = text.lines().filter(|l| !l.starts_with('#'));
    assert_eq!(
        lines.next().unwrap(),
        "fichier\tfonction\tgarde\toutil\tportee\tstatut\tnote"
    );
    for line in lines.filter(|l| !l.trim().is_empty()) {
        let c: Vec<&str> = line.split('\t').collect();
        assert_eq!(c.len(), 7, "7 colonnes attendues : {line}");
        let row = Row {
            guard: c[2].into(),
            tool: c[3].into(),
            scope: c[4].into(),
            status: c[5].into(),
            note: c[6].into(),
        };
        let key = (c[0].to_string(), c[1].to_string());
        assert!(rows.insert(key, row).is_none(), "ligne en double : {line}");
    }
    rows
}

#[test]
fn every_server_function_has_a_decision_and_the_table_lists_nothing_else() {
    let in_code = server_functions();
    let in_table = table();
    assert!(
        in_code.len() >= 100,
        "analyse du code trop pauvre : {}",
        in_code.len()
    );
    let undecided: Vec<_> = in_code
        .iter()
        .filter(|k| !in_table.contains_key(*k))
        .collect();
    assert!(
        undecided.is_empty(),
        "fonctions serveur sans decision dans tests/mcp/coverage.tsv : {undecided:?}"
    );
    let stale: Vec<_> = in_table.keys().filter(|k| !in_code.contains(*k)).collect();
    assert!(
        stale.is_empty(),
        "lignes de coverage.tsv sans fonction serveur : {stale:?}"
    );
}

#[test]
fn every_row_is_well_formed() {
    for ((file, name), r) in table() {
        let at = format!("{file}::{name}");
        assert!(
            matches!(r.guard.as_str(), "admin" | "user" | "session"),
            "{at} : garde {}",
            r.guard
        );
        match r.status.as_str() {
            "couvert" => {
                assert!(!r.tool.is_empty(), "{at} : outil manquant");
                assert!(
                    matches!(
                        r.scope.as_str(),
                        "lecture" | "pilotage" | "configuration" | "administration"
                    ),
                    "{at} : portee {}",
                    r.scope
                );
            }
            "exclu" => {
                assert!(
                    r.tool.is_empty() && r.scope.is_empty(),
                    "{at} : un exclu n'a ni outil ni portee"
                );
                assert!(
                    !r.note.is_empty(),
                    "{at} : la raison de l'exclusion est obligatoire"
                );
            }
            "a_decider" => assert!(!r.note.is_empty(), "{at} : dire ce qui reste a decider"),
            other => panic!("{at} : statut inconnu {other}"),
        }
    }
}

#[test]
fn nothing_that_changes_accounts_or_sends_messages_is_ever_covered() {
    // Definitive exclusions of `ROADMAP-mcp.md` section 4: accounts, passwords, messaging, access tokens.
    for ((file, name), r) in table() {
        let sensitive = file == "messaging.rs"
            || file == "android.rs"
            || file == "mcp_admin.rs"
            || matches!(
                name.as_str(),
                "list_users"
                    | "save_user"
                    | "delete_user"
                    | "change_password"
                    | "login"
                    | "logout"
                    | "whoami"
            );
        if sensitive {
            assert_eq!(
                r.status, "exclu",
                "{file}::{name} ne doit jamais etre expose"
            );
        }
    }
}
