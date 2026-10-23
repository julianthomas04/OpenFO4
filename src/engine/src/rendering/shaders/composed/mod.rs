//! Registry of every shader module composed from repo-owned wrappers and install-time translated parts.

use super::composition::ComposedShaderSource;

pub mod displacement;
pub mod dof;
pub mod grass;
pub mod ssr;
pub mod water;

const AREAS: &[&[&ComposedShaderSource]] = &[
    dof::MODULES,
    ssr::MODULES,
    water::MODULES,
    displacement::MODULES,
    grass::MODULES,
];

/// Every composed module; validated once at startup.
pub fn all() -> Vec<&'static ComposedShaderSource> {
    AREAS.iter().flat_map(|area| area.iter().copied()).collect()
}

#[cfg(test)]
pub fn stub_translated_shaders() -> super::translated::TranslatedShaders {
    let stubs = [
        dof::stubs(),
        ssr::stubs(),
        water::stubs(),
        displacement::stubs(),
        grass::stubs(),
    ]
    .concat();
    super::translated::TranslatedShaders::from_stubs(stubs)
}

/// Entry points (name and stage) of `source` composed with the synthetic stubs; the entry
/// points live in the repo-owned wrappers, so the stubs show every one the pipelines can ask for.
#[cfg(test)]
pub fn entry_points(source: &ComposedShaderSource) -> Vec<(String, naga::ShaderStage)> {
    let wgsl = source.compose(&stub_translated_shaders()).unwrap();
    let module = naga::front::wgsl::parse_str(&wgsl).unwrap();
    module
        .entry_points
        .iter()
        .map(|entry| (entry.name.clone(), entry.stage))
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::rendering::device::REQUIRED_FEATURES;
    use crate::rendering::shaders::composition::ShaderPart;
    use crate::rendering::shaders::startup::{capabilities_of, validate_all};
    use crate::rendering::shaders::translated::{ShaderStage, TranslatedShaders};
    use serde::Deserialize;

    /// The checked-in installer manifest that decides which blobs the install-time
    /// translation produces; the engine must only ask for names it lists.
    const INSTALLER_MANIFEST: &str =
        include_str!("../../../../../installer/shader_translator/shader_manifest.json");

    #[derive(Deserialize)]
    struct InstallerManifest {
        shaders: Vec<InstallerShader>,
    }

    #[derive(Deserialize)]
    struct InstallerShader {
        name: String,
        stage: ShaderStage,
    }

    fn installer_stages() -> Vec<(String, ShaderStage)> {
        let manifest: InstallerManifest = serde_json::from_str(INSTALLER_MANIFEST).unwrap();
        manifest
            .shaders
            .into_iter()
            .map(|shader| (shader.name, shader.stage))
            .collect()
    }

    fn translated_parts() -> Vec<(&'static str, ShaderStage)> {
        all()
            .iter()
            .flat_map(|module| module.parts)
            .filter_map(|part| match part {
                ShaderPart::Translated { name, stage } => Some((*name, *stage)),
                ShaderPart::Static(_) => None,
            })
            .collect()
    }

    #[test]
    fn every_module_validates_against_synthetic_translated_stubs() {
        validate_all(
            &all(),
            &stub_translated_shaders(),
            capabilities_of(REQUIRED_FEATURES),
        )
        .unwrap();
    }

    #[test]
    fn every_translated_part_is_in_the_installer_manifest_with_the_same_stage() {
        let manifest = installer_stages();
        for (name, stage) in translated_parts() {
            let listed = manifest
                .iter()
                .find(|(listed, _)| listed == name)
                .unwrap_or_else(|| panic!("{name} is not in shader_manifest.json"));
            assert_eq!(listed.1, stage, "{name}");
        }
    }

    #[test]
    fn every_installer_manifest_shader_is_used_by_a_module() {
        let used = translated_parts();
        for (name, _) in installer_stages() {
            assert!(
                used.iter().any(|(part, _)| *part == name),
                "{name} is translated at install time but no module uses it"
            );
        }
    }

    #[test]
    fn module_directives_only_open_a_module_and_the_prelude_is_never_repeated() {
        for module in all() {
            for (index, part) in module.parts.iter().enumerate() {
                let ShaderPart::Static(text) = part else {
                    continue;
                };
                assert!(
                    !text.contains("diagnostic("),
                    "{}: the prelude owns the diagnostic directive",
                    module.label
                );
                let first = text.trim_start();
                let opens_with_directive =
                    first.starts_with("enable ") || first.starts_with("requires ");
                assert!(
                    index == 0 || !opens_with_directive,
                    "{}: part {index} starts with a directive after declarations",
                    module.label
                );
            }
        }
    }

    #[test]
    fn module_labels_are_unique() {
        let mut labels: Vec<_> = all().iter().map(|module| module.label).collect();
        labels.sort_unstable();
        let count = labels.len();
        labels.dedup();
        assert_eq!(labels.len(), count);
    }

    /// Developer check against a real install: set `OPENFO4_INSTALL_DIR` to an install whose
    /// `assets` folder holds the `shaders` folder (installer output); `OPENFO4_SHADER_LABELS` optionally restricts the
    /// modules to those whose label starts with one of the comma separated prefixes.
    #[test]
    #[ignore = "needs the installer's translated shaders folder"]
    fn every_module_validates_against_the_installed_shaders() {
        let install = std::env::var("OPENFO4_INSTALL_DIR").expect("OPENFO4_INSTALL_DIR");
        let assets = std::path::Path::new(&install).join("assets");
        let translated = TranslatedShaders::load(&assets).unwrap();
        let prefixes = std::env::var("OPENFO4_SHADER_LABELS").unwrap_or_default();
        let selected: Vec<_> = all()
            .into_iter()
            .filter(|module| {
                prefixes.is_empty()
                    || prefixes
                        .split(',')
                        .any(|prefix| module.label.starts_with(prefix))
            })
            .collect();
        let started = std::time::Instant::now();
        validate_all(&selected, &translated, capabilities_of(REQUIRED_FEATURES)).unwrap();
        eprintln!(
            "validated {} modules in {:?}",
            selected.len(),
            started.elapsed()
        );
    }
}
