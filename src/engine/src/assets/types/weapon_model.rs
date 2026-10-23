//! Reads the assembly data of a converted weapon model from disk.

use std::path::{Path, PathBuf};

use super::glb_key::glb_key;
use super::weapon_assembly::ConnectInfo;

#[derive(Debug, thiserror::Error)]
#[error("weapon model '{}' unreadable: {source}", path.display())]
pub struct ConnectReadError {
    pub path: PathBuf,
    #[source]
    pub source: gltf::Error,
}

/// Reads only the glTF document of `model_path` (a database model path), so
/// the mesh-less dummy base of a gun is accepted.
pub fn read_connect_info(
    asset_root: &Path,
    model_path: &str,
) -> Result<ConnectInfo, ConnectReadError> {
    let path = asset_root.join(glb_key(model_path));
    match gltf::Gltf::open(&path) {
        Ok(gltf) => Ok(ConnectInfo::from_document(&gltf.document)),
        Err(source) => Err(ConnectReadError { path, source }),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_missing_model_names_its_path() {
        let error = read_connect_info(Path::new("no-such-root"), r"weapons\gun.nif").unwrap_err();
        assert!(error.to_string().contains("gun.glb"), "{error}");
    }

    #[test]
    fn a_meshless_model_is_read_without_geometry() {
        let directory =
            std::env::temp_dir().join(format!("openfo4-weapon-model-{}", std::process::id()));
        let meshes = directory.join("meshes").join("weapons");
        std::fs::create_dir_all(&meshes).unwrap();
        let doc = serde_json::json!({
            "asset": {"version": "2.0"},
            "scenes": [{"nodes": [0]}],
            "nodes": [{"name": "Root", "extras": {"connect_parents": {"v": 1, "points": [
                {"name": "P-Receiver", "translation": [0, 0, 0],
                 "rotation": [0, 0, 0, 1], "scale": 1.0}]}}}]
        });
        std::fs::write(meshes.join("dummy.glb"), doc.to_string()).unwrap();
        let info = read_connect_info(&directory, r"weapons\dummy.nif").unwrap();
        std::fs::remove_dir_all(&directory).unwrap();
        assert!(!info.has_geometry);
        assert_eq!(info.parents[0].name, "P-Receiver");
    }
}
