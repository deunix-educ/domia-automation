// SPDX-License-Identifier: AGPL-3.0-or-later
//! Re-runs the compilation when a migration changes (`sqlx::migrate!` embeds them in the binary).

fn main() {
    println!("cargo:rerun-if-changed=migrations");
}
