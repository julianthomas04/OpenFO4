//! Case-insensitive lookup of an asset key below the asset root: the archives store mixed-case
//! names while the contract keys are lower case, and the filesystem may be case-sensitive.

use std::io;
use std::path::Path;

/// The path below `root` that `key` names, with every component matched ignoring ASCII case
/// and joined with `/`.
pub fn resolve_below(root: &Path, key: &str) -> io::Result<String> {
    let mut current = root.to_path_buf();
    let mut parts = Vec::new();
    for component in key.split(['\\', '/']).filter(|part| !part.is_empty()) {
        let name = if current.join(component).exists() {
            component.to_owned()
        } else {
            find_ignoring_case(&current, component)?
        };
        current.push(&name);
        parts.push(name);
    }
    Ok(parts.join("/"))
}

fn find_ignoring_case(directory: &Path, wanted: &str) -> io::Result<String> {
    for entry in std::fs::read_dir(directory)? {
        let name = entry?.file_name().to_string_lossy().into_owned();
        if name.eq_ignore_ascii_case(wanted) {
            return Ok(name);
        }
    }
    Err(io::Error::new(
        io::ErrorKind::NotFound,
        format!("'{wanted}' not found in {}", directory.display()),
    ))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_key_resolves_through_directories_of_a_different_case() {
        let dir = tempfile::tempdir().unwrap();
        let nested = dir.path().join("Textures").join("Interface");
        std::fs::create_dir_all(&nested).unwrap();
        std::fs::write(nested.join("WorldMap_d.dds"), b"dds").unwrap();

        let relative = resolve_below(dir.path(), "textures\\interface\\worldmap_d.dds").unwrap();

        assert!(dir.path().join(&relative).is_file(), "{relative}");
        assert!(relative.eq_ignore_ascii_case("textures/interface/worldmap_d.dds"));
    }

    #[test]
    fn a_missing_file_is_a_not_found_error_naming_the_component() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::create_dir_all(dir.path().join("textures")).unwrap();

        let error = resolve_below(dir.path(), "textures\\worldmap_d.dds").unwrap_err();

        assert_eq!(error.kind(), io::ErrorKind::NotFound);
        assert!(error.to_string().contains("worldmap_d.dds"));
    }

    #[test]
    fn a_missing_root_is_an_error_not_an_empty_path() {
        let dir = tempfile::tempdir().unwrap();
        let missing = dir.path().join("nowhere");
        assert!(resolve_below(&missing, "textures\\a.dds").is_err());
    }
}
