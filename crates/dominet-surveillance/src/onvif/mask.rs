// SPDX-License-Identifier: AGPL-3.0-or-later
//! Masquage des secrets dans les URL et messages qui viennent d'une camera.

/// Cache : le mot de passe d'une URL `schema://utilisateur:motdepasse@hote`, la valeur de tout `password=`/`pass=`/`pwd=`
/// (jusqu'a `&`, `_`, `/`, un espace ou la fin), et chaque occurrence des secrets connus de `known`.
#[must_use]
pub fn mask_secrets(text: &str, known: &[&str]) -> String {
    let mut out = text.to_string();
    for secret in known.iter().filter(|s| !s.is_empty()) {
        out = out.replace(secret, "***");
    }
    out = mask_userinfo(&out);
    for key in ["password=", "passwd=", "pass=", "pwd="] {
        out = mask_value(&out, key);
    }
    out
}

fn mask_userinfo(s: &str) -> String {
    let Some(start) = s.find("://") else {
        return s.to_string();
    };
    let after = start + 3;
    let rest = &s[after..];
    let end = rest.find(['/', '?', ' ']).unwrap_or(rest.len());
    let authority = &rest[..end];
    // Le dernier `@` separe l'identifiant de l'hote (un mot de passe peut contenir `@` non echappe).
    let Some(at) = authority.rfind('@') else {
        return s.to_string();
    };
    let Some(colon) = authority[..at].find(':') else {
        return s.to_string();
    };
    format!(
        "{}{}:***{}",
        &s[..after],
        &authority[..colon],
        &s[after + at..]
    )
}

fn mask_value(s: &str, key: &str) -> String {
    let lower = s.to_ascii_lowercase();
    let mut out = String::with_capacity(s.len());
    let mut from = 0;
    while let Some(i) = lower[from..].find(key) {
        let value_start = from + i + key.len();
        out.push_str(&s[from..value_start]);
        let value_len = s[value_start..]
            .find(['&', '_', '/', ' ', '"', '\''])
            .unwrap_or(s.len() - value_start);
        if value_len > 0 {
            out.push_str("***");
        }
        from = value_start + value_len;
    }
    out.push_str(&s[from..]);
    out
}

#[cfg(test)]
mod tests {
    use super::mask_secrets;

    #[test]
    fn userinfo_and_query_style_passwords_are_hidden() {
        assert_eq!(
            mask_secrets("rtsp://admin:hunter2@192.168.1.29:554/stream1", &[]),
            "rtsp://admin:***@192.168.1.29:554/stream1"
        );
        assert_eq!(
            mask_secrets(
                "rtsp://192.168.1.29:8899/user=admin_password=Zx9!_channel=1_stream=0.sdp",
                &[]
            ),
            "rtsp://192.168.1.29:8899/user=admin_password=***_channel=1_stream=0.sdp"
        );
        assert_eq!(
            mask_secrets("http://h/x?pass=abc&u=1", &[]),
            "http://h/x?pass=***&u=1"
        );
    }

    #[test]
    fn known_secrets_are_hidden_even_when_they_contain_separators() {
        // Un mot de passe qui contient `_` : le masquage par motif s'arreterait trop tot, le secret connu couvre le reste.
        let masked = mask_secrets(
            "rtsp://h/user=a_password=my_secret_x_channel=1",
            &["my_secret_x"],
        );
        assert!(!masked.contains("secret"), "{masked}");
        assert!(masked.contains("channel=1"));
        // Un `@` dans le mot de passe d'une URL.
        assert_eq!(
            mask_secrets("rtsp://u:p@ss@host/a", &[]),
            "rtsp://u:***@host/a"
        );
    }

    #[test]
    fn text_without_secrets_is_unchanged() {
        assert_eq!(
            mask_secrets("rtsp://192.168.1.60:554/ch0", &[]),
            "rtsp://192.168.1.60:554/ch0"
        );
        assert_eq!(
            mask_secrets("Sender not authorized", &[]),
            "Sender not authorized"
        );
    }
}
