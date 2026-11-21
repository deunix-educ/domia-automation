// SPDX-License-Identifier: AGPL-3.0-or-later
//! Adresses internes : cible de retour apres connexion (filtree contre les redirections ouvertes).

/// Ne garde la cible `next` que si c'est un chemin interne sur ce site ; sinon `/`.
///
/// Refuse tout ce qui pourrait mener hors du site : URL absolue (`https://...`), adresse sans schema (`//hote`),
/// barre oblique inverse (certains navigateurs la traitent comme `/`), caracteres de controle.
#[must_use]
pub fn safe_next(next: &str) -> String {
    let ok = next.starts_with('/')
        && !next.starts_with("//")
        && !next.contains('\\')
        && !next.chars().any(char::is_control)
        && !next.starts_with("/login");
    if ok {
        next.to_string()
    } else {
        "/".to_string()
    }
}

/// Vrai si `path` est servi par le serveur HORS de l'interface (documentation, Grafana) : le routeur du navigateur ne le
/// connait pas et afficherait « Page introuvable » ; il faut un vrai chargement de page.
#[must_use]
pub fn leaves_app(path: &str) -> bool {
    let path = path.split(['?', '#']).next().unwrap_or(path);
    ["/docs", "/grafana"].iter().any(|p| {
        path == *p
            || path
                .strip_prefix(p)
                .is_some_and(|rest| rest.starts_with('/'))
    })
}

/// Adresse de la page de connexion qui ramenera ensuite vers `target` (rien si c'est l'accueil).
#[must_use]
pub fn login_url(target: &str) -> String {
    let target = safe_next(target);
    if target == "/" {
        "/login".to_string()
    } else {
        format!("/login?next={}", encode(&target))
    }
}

/// Encodage pourcent des octets hors alphabet non reserve (suffisant pour une valeur de parametre).
fn encode(s: &str) -> String {
    use std::fmt::Write;
    let mut out = String::with_capacity(s.len());
    for b in s.bytes() {
        if b.is_ascii_alphanumeric() || matches!(b, b'-' | b'_' | b'.' | b'~') {
            out.push(char::from(b));
        } else {
            let _ = write!(out, "%{b:02X}");
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn pages_outside_the_interface_need_a_full_load() {
        for out in ["/docs", "/docs/", "/docs/en/mqtt/?x=1", "/grafana/d/abc#p"] {
            assert!(leaves_app(out), "{out}");
        }
        for inside in ["/", "/admin/logs", "/docsx", "/admin/docs/"] {
            assert!(!leaves_app(inside), "{inside}");
        }
    }

    #[test]
    fn only_internal_paths_are_kept() {
        for ok in ["/", "/account", "/admin/users?x=1", "/a%20b"] {
            assert_eq!(safe_next(ok), ok);
        }
        for bad in [
            "",
            "account",
            "//evil.example",
            "https://evil.example",
            "/\\evil.example",
            "/a\nb",
            "javascript:alert(1)",
            "/login",
            "/login?next=/",
        ] {
            assert_eq!(safe_next(bad), "/", "{bad:?}");
        }
    }

    #[test]
    fn the_login_url_carries_an_encoded_target() {
        assert_eq!(login_url("/"), "/login");
        assert_eq!(login_url("//evil"), "/login");
        assert_eq!(login_url("/account"), "/login?next=%2Faccount");
        assert_eq!(
            login_url("/admin/users?a=b&c=d"),
            "/login?next=%2Fadmin%2Fusers%3Fa%3Db%26c%3Dd"
        );
    }
}
