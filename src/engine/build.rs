//! Hands the cargo profile family to the engine. `PROFILE` is `debug` or `release` whatever the
//! profile's settings are (a custom profile reports the one it inherits from), unlike
//! `debug_assertions`, which any profile can override.

use std::env;

fn main() {
    println!("cargo:rerun-if-changed=build.rs");
    println!("cargo:rerun-if-env-changed=PROFILE");
    let profile = env::var("PROFILE").expect("set by cargo");
    assert!(
        matches!(profile.as_str(), "debug" | "release"),
        "unsupported cargo PROFILE '{profile}': expected 'debug' or 'release'"
    );
    println!("cargo:rustc-env=OPENFO4_BUILD_PROFILE={profile}");
}
