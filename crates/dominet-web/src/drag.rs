// SPDX-License-Identifier: AGPL-3.0-or-later
//! Deplacement d'un widget a la souris ou au doigt (administrateurs, R2 lot 5).
//!
//! Evenements de **pointeur** (souris, doigt et stylet de la meme facon, contrairement au glisser-deposer HTML5) avec capture :
//! le widget suit le pointeur par une transformation posee par la CSSOM (la CSP interdit l'attribut `style`, pas la CSSOM) et un
//! cadre en pointilles montre la case visee. Au relachement, la page demande le deplacement au serveur, qui a le dernier mot
//! (grille, chevauchement) ; si le serveur refuse, le widget revient en place.
//!
//! Seulement avec la grille complete de 12 colonnes (ecrans larges) : sur les autres tailles les widgets se placent a la suite et leur
//! position d'ecran n'est plus celle de la grille.

use leptos::prelude::*;

/// Ce qu'il faut savoir d'un widget pour le deplacer.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct DragSpec {
    pub id: i64,
    pub col: i16,
    pub row: i16,
    pub width: i16,
    pub height: i16,
}

/// Case de grille choisie au relachement, si elle differe de la position d'origine.
pub type Target = (i16, i16);

#[cfg(feature = "hydrate")]
mod imp {
    use wasm_bindgen::JsCast;
    use web_sys::{HtmlElement, PointerEvent};

    use super::{DragSpec, Target};

    /// Largeur minimale de la fenetre pour la grille complete (voir `main.css`).
    const MIN_WIDTH: f64 = 1201.0;
    const ROW_HEIGHT: f64 = 120.0;
    const COLUMNS: i16 = 12;
    const MAX_ROW: i16 = 99;

    #[derive(Debug)]
    pub struct Session {
        widget: HtmlElement,
        header: HtmlElement,
        grid: HtmlElement,
        ghost: Option<HtmlElement>,
        pointer: i32,
        spec: DragSpec,
        start: (f64, f64),
        origin: (f64, f64),
        target: Target,
    }

    fn px(style: &web_sys::CssStyleDeclaration, name: &str) -> f64 {
        style
            .get_property_value(name)
            .ok()
            .and_then(|v| v.trim_end_matches("px").trim().parse::<f64>().ok())
            .unwrap_or(0.0)
    }

    /// Pas d'une colonne et d'une ligne, et marges de la grille, lus dans les styles calcules.
    fn metrics(grid: &HtmlElement) -> Option<(f64, f64, f64, f64, f64)> {
        let window = web_sys::window()?;
        let style = window.get_computed_style(grid).ok()??;
        let rect = grid.get_bounding_client_rect();
        let (pad_l, pad_r, pad_t) = (
            px(&style, "padding-left"),
            px(&style, "padding-right"),
            px(&style, "padding-top"),
        );
        let gap = px(&style, "column-gap");
        let column =
            (rect.width() - pad_l - pad_r - gap * f64::from(COLUMNS - 1)) / f64::from(COLUMNS);
        Some((
            column + gap,
            ROW_HEIGHT + px(&style, "row-gap"),
            pad_l,
            pad_t,
            rect.left(),
        ))
    }

    pub fn down(ev: &PointerEvent, spec: DragSpec) -> Option<Session> {
        let window = web_sys::window()?;
        if ev.button() != 0 || window.inner_width().ok()?.as_f64()? < MIN_WIDTH {
            return None;
        }
        let header = ev.current_target()?.dyn_into::<HtmlElement>().ok()?;
        let widget = header.parent_element()?.dyn_into::<HtmlElement>().ok()?;
        let grid = widget.parent_element()?.dyn_into::<HtmlElement>().ok()?;
        let ghost = grid
            .query_selector("#vc-drop-ghost")
            .ok()?
            .and_then(|e| e.dyn_into::<HtmlElement>().ok());
        header.set_pointer_capture(ev.pointer_id()).ok()?;
        let rect = widget.get_bounding_client_rect();
        let _ = widget.class_list().add_1("vc-widget--dragging");
        Some(Session {
            widget,
            header,
            grid,
            ghost,
            pointer: ev.pointer_id(),
            spec,
            start: (f64::from(ev.client_x()), f64::from(ev.client_y())),
            origin: (rect.left(), rect.top()),
            target: (spec.col, spec.row),
        })
    }

    #[allow(clippy::cast_possible_truncation)]
    fn cell(value: f64, pitch: f64, max: i16) -> i16 {
        ((value / pitch).round() as i16).clamp(0, max)
    }

    pub fn moved(s: &mut Session, ev: &PointerEvent) {
        if ev.pointer_id() != s.pointer {
            return;
        }
        let (dx, dy) = (
            f64::from(ev.client_x()) - s.start.0,
            f64::from(ev.client_y()) - s.start.1,
        );
        let _ = s
            .widget
            .style()
            .set_property("transform", &format!("translate({dx}px, {dy}px)"));
        let Some((pitch_x, pitch_y, pad_l, pad_t, _)) = metrics(&s.grid) else {
            return;
        };
        // Le haut-gauche du widget deplace, mesure dans le repere de la grille (qui peut avoir defile).
        let grid = s.grid.get_bounding_client_rect();
        let x = s.origin.0 + dx - grid.left() - pad_l;
        let y = s.origin.1 + dy - grid.top() - pad_t;
        s.target = (
            cell(x, pitch_x, COLUMNS - s.spec.width),
            cell(y, pitch_y, MAX_ROW),
        );
        if let Some(g) = &s.ghost {
            let st = g.style();
            let _ = st.set_property(
                "grid-column",
                &format!("{} / span {}", s.target.0 + 1, s.spec.width),
            );
            let _ = st.set_property(
                "grid-row",
                &format!("{} / span {}", s.target.1 + 1, s.spec.height),
            );
            let _ = st.set_property("display", "block");
        }
    }

    fn release(s: &Session) {
        let _ = s.header.release_pointer_capture(s.pointer);
        let _ = s.widget.style().remove_property("transform");
        let _ = s.widget.class_list().remove_1("vc-widget--dragging");
        if let Some(g) = &s.ghost {
            let _ = g.style().set_property("display", "none");
        }
    }

    /// Termine le geste : `Some(case)` si le widget doit changer de place.
    pub fn up(s: Session, ev: &PointerEvent) -> Option<Target> {
        let mut s = s;
        moved(&mut s, ev);
        release(&s);
        (s.target != (s.spec.col, s.spec.row)).then_some(s.target)
    }

    pub fn cancel(s: &Session) {
        release(s);
    }
}

#[cfg(not(feature = "hydrate"))]
mod imp {
    #[derive(Debug)]
    pub struct Session;
}

pub use imp::Session;

/// Geste en cours (une seule a la fois), propre au widget. Dans le navigateur, les elements du DOM ne sont pas `Send` : stockage
/// local. **Cote serveur le stockage doit rester ordinaire** : un stockage local cree pendant le rendu serait libere par un autre
/// fil et paniquerait (`SendWrapper`), ce qui figerait la page.
#[cfg(feature = "hydrate")]
pub type Handle = StoredValue<Option<Session>, LocalStorage>;
#[cfg(not(feature = "hydrate"))]
pub type Handle = StoredValue<Option<Session>>;

/// Cree la poignee d'un widget.
#[must_use]
pub fn handle() -> Handle {
    #[cfg(feature = "hydrate")]
    return StoredValue::new_local(None);
    #[cfg(not(feature = "hydrate"))]
    StoredValue::new(None)
}

/// Debut d'un geste sur l'en-tete du widget.
#[cfg(feature = "hydrate")]
pub fn down(h: Handle, ev: &web_sys::PointerEvent, spec: DragSpec) {
    if let Some(s) = imp::down(ev, spec) {
        h.set_value(Some(s));
    }
}

/// Mouvement du pointeur.
#[cfg(feature = "hydrate")]
pub fn moved(h: Handle, ev: &web_sys::PointerEvent) {
    h.update_value(|s| {
        if let Some(s) = s {
            imp::moved(s, ev);
        }
    });
}

/// Fin du geste : la case visee, si elle change.
#[cfg(feature = "hydrate")]
#[must_use]
pub fn up(h: Handle, ev: &web_sys::PointerEvent) -> Option<Target> {
    let s = h.try_update_value(Option::take).flatten()?;
    imp::up(s, ev)
}

/// Geste interrompu (perte de capture, annulation systeme).
#[cfg(feature = "hydrate")]
pub fn cancel(h: Handle) {
    if let Some(s) = h.try_update_value(Option::take).flatten() {
        imp::cancel(&s);
    }
}
