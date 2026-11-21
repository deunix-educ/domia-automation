// SPDX-License-Identifier: AGPL-3.0-or-later
//! Assainissement du HTML saisi par les administrateurs (widget `text`, ADR 0022).
//!
//! Liste blanche (`ammonia`) : mise en forme, listes, tableaux, liens et images. Ni script, ni iframe, ni formulaire, ni
//! attribut `style` ni gestionnaire d'evenement ; les liens http(s)/mailto/tel seulement, ouverts avec
//! `rel="noopener noreferrer"`. La CSP bloque deja les scripts, mais pas le detournement de formulaires : d'ou la liste
//! blanche plutot qu'une simple interdiction des scripts.

use std::collections::HashSet;

/// HTML sur : a utiliser a l'enregistrement **et** avant tout affichage (le contenu en base n'est pas fiable).
#[must_use]
pub fn sanitize_widget_html(dirty: &str) -> String {
    let mut builder = ammonia::Builder::default();
    builder
        .url_schemes(HashSet::from(["http", "https", "mailto", "tel"]))
        .link_rel(Some("noopener noreferrer"))
        // `target` est retire par defaut : un lien externe reste dans l'onglet (pas de fenetre piegee).
        .strip_comments(true);
    builder.clean(dirty).to_string()
}

/// Texte brut -> HTML : tout est echappe, les retours a la ligne deviennent `<br>`.
#[must_use]
pub fn plain_text_to_html(text: &str) -> String {
    ammonia::clean_text(text)
        .replace("&NewLine;", "<br>")
        .replace('\n', "<br>")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn formatting_survives() {
        let out =
            sanitize_widget_html("<p>Bonjour <b>le</b> <em>monde</em></p><ul><li>un</li></ul>");
        assert_eq!(
            out,
            "<p>Bonjour <b>le</b> <em>monde</em></p><ul><li>un</li></ul>"
        );
    }

    #[test]
    fn scripts_frames_forms_and_handlers_are_removed() {
        let dirty = r#"<script>alert(1)</script><iframe src="http://x"></iframe><form action="http://evil"><input name="p"></form><p onclick="x()" style="color:red">t</p><img src="x" onerror="y()">"#;
        let out = sanitize_widget_html(dirty);
        for banned in [
            "script", "iframe", "form", "input", "onclick", "onerror", "style", "alert",
        ] {
            assert!(!out.contains(banned), "{banned} dans {out}");
        }
        assert!(out.contains("<p>t</p>"), "{out}");
    }

    #[test]
    fn dangerous_link_schemes_are_removed_and_links_are_isolated() {
        let out = sanitize_widget_html(
            r#"<a href="javascript:alert(1)">a</a><a href="data:text/html,x">b</a><a href="https://ok.example/">c</a>"#,
        );
        assert!(
            !out.contains("javascript") && !out.contains("data:"),
            "{out}"
        );
        assert!(
            out.contains(r#"href="https://ok.example/""#)
                && out.contains(r#"rel="noopener noreferrer""#),
            "{out}"
        );
    }

    #[test]
    fn comments_and_empty_input_are_harmless() {
        assert_eq!(sanitize_widget_html("<!-- secret -->x"), "x");
        assert_eq!(sanitize_widget_html(""), "");
    }
}
