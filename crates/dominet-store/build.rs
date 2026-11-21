// SPDX-License-Identifier: AGPL-3.0-or-later
//! Relance la compilation quand une migration change (`sqlx::migrate!` les embarque dans le binaire).

fn main() {
    println!("cargo:rerun-if-changed=migrations");
}
