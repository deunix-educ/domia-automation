// SPDX-License-Identifier: AGPL-3.0-or-later
//! Icones fixes de l'interface (marque, repli du menu, plein ecran...), identiques cote serveur et navigateur.
//!
//! Extraites de Lucide v1.21.0 (licence ISC, voir `data/LUCIDE-LICENSE.txt`). Les icones de la navigation, elles,
//! viennent de la base et sont rendues cote serveur (`icons.rs`).

macro_rules! svg {
    ($size:literal, $inner:literal) => {
        concat!(
            r#"<svg xmlns="http://www.w3.org/2000/svg" width=""#, $size, r#"" height=""#, $size,
            r#"" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round" class="vc-icon" aria-hidden="true" focusable="false">"#,
            $inner, "</svg>"
        )
    };
}

pub const BRAND: &str = svg!(
    "22",
    r#"<path d="M15 21v-8a1 1 0 0 0-1-1h-4a1 1 0 0 0-1 1v8"/><path d="M3 10a2 2 0 0 1 .709-1.528l7-6a2 2 0 0 1 2.582 0l7 6A2 2 0 0 1 21 10v9a2 2 0 0 1-2 2H5a2 2 0 0 1-2-2z"/>"#
); // house
pub const DASHBOARD: &str = svg!(
    "16",
    r#"<rect width="7" height="7" x="3" y="3" rx="1"/><rect width="7" height="7" x="14" y="3" rx="1"/><rect width="7" height="7" x="14" y="14" rx="1"/><rect width="7" height="7" x="3" y="14" rx="1"/>"#
); // layout-grid
pub const COLLAPSE: &str = svg!("14", r#"<path d="m15 18-6-6 6-6"/>"#); // chevron-left
pub const EXPAND: &str = svg!("18", r#"<path d="m9 18 6-6-6-6"/>"#); // chevron-right
pub const MENU: &str = svg!(
    "20",
    r#"<path d="M4 5h16"/><path d="M4 12h16"/><path d="M4 19h16"/>"#
); // menu
pub const FULLSCREEN: &str = svg!(
    "16",
    r#"<path d="M8 3H5a2 2 0 0 0-2 2v3"/><path d="M21 8V5a2 2 0 0 0-2-2h-3"/><path d="M3 16v3a2 2 0 0 0 2 2h3"/><path d="M16 21h3a2 2 0 0 0 2-2v-3"/>"#
); // maximize
pub const LOGOUT: &str = svg!(
    "14",
    r#"<path d="m16 17 5-5-5-5"/><path d="M21 12H9"/><path d="M9 21H5a2 2 0 0 1-2-2V5a2 2 0 0 1 2-2h4"/>"#
); // log-out
pub const USER: &str = svg!(
    "16",
    r#"<path d="M19 21v-2a4 4 0 0 0-4-4H9a4 4 0 0 0-4 4v2"/><circle cx="12" cy="7" r="4"/>"#
); // user
pub const LOGIN: &str = svg!(
    "28",
    r#"<path d="M15 3h4a2 2 0 0 1 2 2v14a2 2 0 0 1-2 2h-4"/><polyline points="10 17 15 12 10 7"/><line x1="15" x2="3" y1="12" y2="12"/>"#
); // log-in
pub const MAP_PIN: &str = svg!(
    "16",
    r#"<path d="M20 10c0 4.993-5.539 10.193-7.399 11.799a1 1 0 0 1-1.202 0C9.539 20.193 4 14.993 4 10a8 8 0 0 1 16 0"/><circle cx="12" cy="10" r="3"/>"#
); // map-pin
pub const LIST: &str = svg!(
    "16",
    r#"<path d="M3 5h.01"/><path d="M3 12h.01"/><path d="M3 19h.01"/><path d="M8 5h13"/><path d="M8 12h13"/><path d="M8 19h13"/>"#
); // list
pub const SLIDERS: &str = svg!(
    "16",
    r#"<path d="M10 5H3"/><path d="M12 19H3"/><path d="M14 3v4"/><path d="M16 17v4"/><path d="M21 12h-9"/><path d="M21 19h-5"/><path d="M21 5h-7"/><path d="M8 10v4"/><path d="M8 12H3"/>"#
); // sliders-horizontal
pub const USERS: &str = svg!(
    "16",
    r#"<path d="M16 21v-2a4 4 0 0 0-4-4H6a4 4 0 0 0-4 4v2"/><path d="M16 3.128a4 4 0 0 1 0 7.744"/><path d="M22 21v-2a4 4 0 0 0-3-3.87"/><circle cx="9" cy="7" r="4"/>"#
); // users
pub const CPU: &str = svg!(
    "13",
    r#"<path d="M12 20v2"/><path d="M12 2v2"/><path d="M17 20v2"/><path d="M17 2v2"/><path d="M2 12h2"/><path d="M2 17h2"/><path d="M2 7h2"/><path d="M20 12h2"/><path d="M20 17h2"/><path d="M20 7h2"/><path d="M7 20v2"/><path d="M7 2v2"/><rect x="4" y="4" width="16" height="16" rx="2"/><rect x="8" y="8" width="8" height="8" rx="1"/>"#
); // cpu
pub const MEMORY: &str = svg!(
    "13",
    r#"<path d="M12.83 2.18a2 2 0 0 0-1.66 0L2.6 6.08a1 1 0 0 0 0 1.83l8.58 3.91a2 2 0 0 0 1.66 0l8.58-3.9a1 1 0 0 0 0-1.83z"/><path d="M2 12a1 1 0 0 0 .58.91l8.6 3.91a2 2 0 0 0 1.65 0l8.58-3.9A1 1 0 0 0 22 12"/><path d="M2 17a1 1 0 0 0 .58.91l8.6 3.91a2 2 0 0 0 1.65 0l8.58-3.9A1 1 0 0 0 22 17"/>"#
); // layers
pub const DISK: &str = svg!(
    "13",
    r#"<path d="M10 16h.01"/><path d="M2.212 11.577a2 2 0 0 0-.212.896V18a2 2 0 0 0 2 2h16a2 2 0 0 0 2-2v-5.527a2 2 0 0 0-.212-.896L18.55 5.11A2 2 0 0 0 16.76 4H7.24a2 2 0 0 0-1.79 1.11z"/><path d="M21.946 12.013H2.054"/><path d="M6 16h.01"/>"#
); // hard-drive
pub const THERMOMETER: &str = svg!(
    "13",
    r#"<path d="M14 4v10.54a4 4 0 1 1-4 0V4a2 2 0 0 1 4 0Z"/>"#
); // thermometer
pub const VIDEO: &str = svg!(
    "14",
    r#"<path d="m16 13 5.223 3.482a.5.5 0 0 0 .777-.416V7.87a.5.5 0 0 0-.752-.432L16 10.5"/><rect x="2" y="6" width="14" height="12" rx="2"/>"#
); // video
pub const ZAP: &str = svg!(
    "14",
    r#"<path d="M4 14a1 1 0 0 1-.78-1.63l9.9-10.2a.5.5 0 0 1 .86.46l-1.92 6.02A1 1 0 0 0 13 10h7a1 1 0 0 1 .78 1.63l-9.9 10.2a.5.5 0 0 1-.86-.46l1.92-6.02A1 1 0 0 0 11 14z"/>"#
); // zap
pub const CHART: &str = svg!(
    "14",
    r#"<path d="M5 21v-6"/><path d="M12 21V3"/><path d="M19 21V9"/>"#
); // bar-chart-2
pub const ALERT: &str = svg!(
    "16",
    r#"<path d="m21.73 18-8-14a2 2 0 0 0-3.48 0l-8 14A2 2 0 0 0 4 21h16a2 2 0 0 0 1.73-3"/><path d="M12 9v4"/><path d="M12 17h.01"/>"#
); // triangle-alert
pub const CHECK: &str = svg!("13", r#"<path d="M20 6 9 17l-5-5"/>"#); // check
pub const CHEVRON_DOWN: &str = svg!("12", r#"<path d="m6 9 6 6 6-6"/>"#); // chevron-down
pub const PLUS: &str = svg!("14", r#"<path d="M5 12h14"/><path d="M12 5v14"/>"#); // plus
pub const PUZZLE: &str = svg!(
    "16",
    r#"<path d="M15.39 4.39a1 1 0 0 0 1.68-.474 2.5 2.5 0 1 1 3.014 3.015 1 1 0 0 0-.474 1.68l1.683 1.682a2.414 2.414 0 0 1 0 3.414L19.61 15.39a1 1 0 0 1-1.68-.474 2.5 2.5 0 1 0-3.014 3.015 1 1 0 0 1 .474 1.68l-1.683 1.682a2.414 2.414 0 0 1-3.414 0L8.61 19.61a1 1 0 0 0-1.68.474 2.5 2.5 0 1 1-3.014-3.015 1 1 0 0 0 .474-1.68l-1.683-1.682a2.414 2.414 0 0 1 0-3.414L4.39 8.61a1 1 0 0 1 1.68.474 2.5 2.5 0 1 0 3.014-3.015 1 1 0 0 1-.474-1.68l1.683-1.682a2.414 2.414 0 0 1 3.414 0z"/>"#
); // puzzle
