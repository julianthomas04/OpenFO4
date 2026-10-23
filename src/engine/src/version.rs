//! The engine's version as the build states it, and the text the game shows for it.

const DEV_VERSION: &str = "0.0.0-dev";

/// The version the build was made for (`OPENFO4_VERSION` at compile time), or the development
/// version of an unversioned build.
pub const VERSION: &str = version_or_dev(option_env!("OPENFO4_VERSION"));

const fn version_or_dev(stated: Option<&'static str>) -> &'static str {
    match stated {
        Some(version) => version,
        None => DEV_VERSION,
    }
}

/// The name of this build's cargo profile family, which `build.rs` states and has checked.
pub fn build_name() -> &'static str {
    build_name_of(env!("OPENFO4_BUILD_PROFILE"))
}

/// The build's name for the cargo profile family `profile`; `build.rs` refuses any other value,
/// so reaching the panic means the two disagree.
fn build_name_of(profile: &str) -> &'static str {
    match profile {
        "debug" => "Development",
        "release" => "Release",
        other => panic!("the build profile '{other}' is neither 'debug' nor 'release'"),
    }
}

/// The text shown in the screen corner for `version`.
pub fn display_text(version: &str) -> String {
    format!("OpenFO4 v{version}")
}

/// The lines of the corner text, top to bottom: the version, then the build.
pub fn overlay_lines() -> [String; 2] {
    [display_text(VERSION), build_display_text(build_name())]
}

/// The text shown below the version for the build named `build`.
pub fn build_display_text(build: &str) -> String {
    format!("Build: {build}")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_stated_version_is_used_as_given() {
        assert_eq!(version_or_dev(Some("1.2.3-rc.1")), "1.2.3-rc.1");
    }

    #[test]
    fn an_unstated_version_is_the_development_version() {
        assert_eq!(version_or_dev(None), "0.0.0-dev");
    }

    #[test]
    fn the_cargo_profiles_map_to_their_build_names() {
        assert_eq!(build_name_of("debug"), "Development");
        assert_eq!(build_name_of("release"), "Release");
        assert_eq!(build_display_text("Development"), "Build: Development");
    }

    #[test]
    #[should_panic(expected = "neither 'debug' nor 'release'")]
    fn any_other_profile_is_refused() {
        build_name_of("profiling");
    }

    #[test]
    fn this_build_has_a_known_name() {
        assert!(["Development", "Release"].contains(&build_name()));
    }

    #[test]
    fn the_display_text_names_the_game_and_prefixes_the_version_with_v() {
        assert_eq!(display_text("0.0.0-dev"), "OpenFO4 v0.0.0-dev");
        assert_eq!(display_text("2.0.1"), "OpenFO4 v2.0.1");
    }
}
