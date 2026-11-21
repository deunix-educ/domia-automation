// SPDX-License-Identifier: AGPL-3.0-or-later
//! URL publiques (WebRTC, R6) adaptees a l'hote par lequel le navigateur est arrive.
//!
//! Les reglages (`MEDIAMTX_WEBRTC_DOMAIN`) contiennent le nom du reseau local (`dominet.local`). Un navigateur qui joint DomiNet par
//! une autre adresse (IP du reseau local, second VPN...) ne sait pas forcement resoudre ce nom : on remplace alors l'hote configure
//! par celui de la requete, **si** l'hote configure est un des hotes propres de l'instance (`ALLOWED_HOSTS`). MediaMTX et DomiNet sont
//! servis par le meme nginx, donc sur le meme hote.

/// Hotes propres a l'instance : `ALLOWED_HOSTS` sans les adresses purement locales ni les jokers.
#[must_use]
pub fn own_hosts(allowed: &[String]) -> Vec<String> {
    allowed
        .iter()
        .map(|h| h.trim().to_ascii_lowercase())
        .filter(|h| {
            !h.is_empty()
                && !matches!(h.as_str(), "localhost" | "127.0.0.1" | "web")
                && !h.starts_with(['*', '.'])
        })
        .collect()
}

/// Hote (sans port) d'un texte `[schema://]hote[:port][/chemin]` ; les crochets d'une adresse IPv6 sont retires.
fn host_of(authority_and_path: &str) -> String {
    let authority = authority_and_path
        .split(['/', '?', '#'])
        .next()
        .unwrap_or("");
    let authority = authority.rsplit_once('@').map_or(authority, |(_, a)| a);
    if let Some(rest) = authority.strip_prefix('[') {
        return rest.split(']').next().unwrap_or("").to_ascii_lowercase();
    }
    authority
        .split(':')
        .next()
        .unwrap_or("")
        .to_ascii_lowercase()
}

/// `configured` avec l'hote de la requete, si l'hote configure est un des notres ; sinon `configured` tel quel.
///
/// Le schema, le port et le chemin configures sont conserves. Une valeur sans schema (`dominet.local:8889`) reste sans schema.
/// `request_host` est l'en-tete `Host` (avec son port eventuel).
#[must_use]
pub fn public_url(configured: &str, request_host: Option<&str>, own_hosts: &[String]) -> String {
    let Some(request_host) = request_host else {
        return configured.to_string();
    };
    let (scheme, rest) = match configured.split_once("://") {
        Some((s, r)) => (Some(s), r),
        None => (None, configured),
    };
    let configured_host = host_of(rest);
    let wanted = host_of(request_host);
    if configured_host.is_empty()
        || wanted.is_empty()
        || wanted == configured_host
        || !own_hosts.contains(&configured_host)
    {
        return configured.to_string();
    }
    // Reste apres l'hote : port et chemin.
    let after_host = if rest.starts_with('[') {
        rest.split_once(']').map_or("", |(_, r)| r)
    } else {
        rest.strip_prefix(configured_host.as_str())
            .or_else(|| {
                // Casse differente dans la configuration : on retire autant de caracteres que l'hote.
                rest.get(configured_host.len()..)
            })
            .unwrap_or("")
    };
    let host = if wanted.contains(':') {
        format!("[{wanted}]")
    } else {
        wanted
    };
    match scheme {
        Some(s) => format!("{s}://{host}{after_host}"),
        None => format!("{host}{after_host}"),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn own() -> Vec<String> {
        own_hosts(&[
            "dominet.local".into(),
            "192.168.1.2".into(),
            "localhost".into(),
            "*.exemple.org".into(),
            ".exemple.net".into(),
            " ".into(),
        ])
    }

    #[test]
    fn own_hosts_drop_local_names_and_wildcards() {
        assert_eq!(own(), ["dominet.local", "192.168.1.2"]);
    }

    #[test]
    fn the_request_host_replaces_a_configured_own_host_and_keeps_scheme_port_and_path() {
        let own = own();
        assert_eq!(
            public_url("http://dominet.local:8889", Some("192.168.1.2"), &own),
            "http://192.168.1.2:8889"
        );
        assert_eq!(
            public_url(
                "https://dominet.local/webrtc",
                Some("192.168.1.2:8443"),
                &own
            ),
            "https://192.168.1.2/webrtc"
        );
        // Sans schema, sans schema.
        assert_eq!(
            public_url("dominet.local:8889", Some("192.168.1.2"), &own),
            "192.168.1.2:8889"
        );
        // Hote de requete IPv6.
        assert_eq!(
            public_url("http://dominet.local:8889", Some("[fd00::1]:3000"), &own),
            "http://[fd00::1]:8889"
        );
    }

    #[test]
    fn the_configured_value_is_kept_in_every_other_case() {
        let own = own();
        // Pas de requete, meme hote, hote configure qui n'est pas le notre (externe, localhost), valeur vide.
        assert_eq!(
            public_url("http://dominet.local:8889", None, &own),
            "http://dominet.local:8889"
        );
        assert_eq!(
            public_url(
                "http://dominet.local:8889",
                Some("dominet.local:3000"),
                &own
            ),
            "http://dominet.local:8889"
        );
        assert_eq!(
            public_url("http://localhost:8889", Some("192.168.1.2"), &own),
            "http://localhost:8889"
        );
        assert_eq!(
            public_url("https://cdn.exemple.com", Some("192.168.1.2"), &own),
            "https://cdn.exemple.com"
        );
        assert_eq!(public_url("", Some("192.168.1.2"), &own), "");
        // Un nom d'hote ne se compare pas selon la casse.
        assert_eq!(
            public_url("http://DOMINET.local:8889", Some("192.168.1.2"), &own),
            "http://192.168.1.2:8889"
        );
    }
}
