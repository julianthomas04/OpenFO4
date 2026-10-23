//! The asset key of a converted model: how a record's `.nif` path names its GLB.

use crate::assets::layout::mesh_key;

const NIF_EXTENSION: &str = ".nif";
const GLB_EXTENSION: &str = ".glb";

/// `meshes\<model_path>` with a trailing `.nif` (any case) turned into `.glb`;
/// only the extension is rewritten, a `.nif` elsewhere in the path is kept.
pub fn glb_key(model_path: &str) -> String {
    let stem_len = model_path
        .len()
        .checked_sub(NIF_EXTENSION.len())
        .filter(|&at| {
            model_path
                .get(at..)
                .is_some_and(|tail| tail.eq_ignore_ascii_case(NIF_EXTENSION))
        });
    match stem_len {
        Some(at) => mesh_key(&format!("{}{GLB_EXTENSION}", &model_path[..at])),
        None => mesh_key(model_path),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_extension_becomes_glb_under_the_meshes_root() {
        assert_eq!(
            glb_key("Landscape\\Grass\\Bent.nif"),
            "meshes\\Landscape\\Grass\\Bent.glb"
        );
    }

    #[test]
    fn the_extension_is_matched_without_regard_to_case() {
        assert_eq!(glb_key("Props\\Crate.NIF"), "meshes\\Props\\Crate.glb");
        assert_eq!(glb_key("Props\\Crate.Nif"), "meshes\\Props\\Crate.glb");
    }

    #[test]
    fn only_the_trailing_extension_is_rewritten() {
        assert_eq!(
            glb_key("Props\\a.nifty\\b.nif"),
            "meshes\\Props\\a.nifty\\b.glb"
        );
        assert_eq!(glb_key("Props\\b.nif.bak"), "meshes\\Props\\b.nif.bak");
    }

    #[test]
    fn a_path_without_the_extension_only_gains_the_root() {
        assert_eq!(glb_key("Props\\Crate"), "meshes\\Props\\Crate");
        assert_eq!(glb_key(""), "meshes\\");
    }

    #[test]
    fn a_multibyte_tail_does_not_panic() {
        assert_eq!(glb_key("Props\\Kräte"), "meshes\\Props\\Kräte");
    }
}
