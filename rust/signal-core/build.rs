// SPDX-License-Identifier: AGPL-3.0-only

fn main() {
    println!("cargo::rerun-if-changed=build.rs");

    if std::env::var("CARGO_CFG_TARGET_OS").as_deref() == Ok("linux") {
        // SQLx owns SQLite worker threads without exposing join handles.
        // Bounded logical shutdown may leave one finishing dependency work,
        // so its Rust code must remain mapped after the plugin is unloaded.
        println!("cargo::rustc-link-arg-cdylib=-Wl,-z,nodelete");
    }
}
