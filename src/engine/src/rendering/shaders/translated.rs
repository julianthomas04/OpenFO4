//! Install-time translated WGSL, a game asset the installer's `shader-translate` step writes to
//! `<asset_root>/shaders/manifest.json` plus one `.wgsl` per entry.

use std::collections::HashMap;
use std::io;
use std::path::{Component, Path, PathBuf};

use serde::Deserialize;

use crate::assets::layout::SHADERS_DIR;

pub const TRANSLATOR_VERSION: u32 = 2;
const RERUN_INSTALLER: &str = "re-run the installer (its `shader-translate` step is enough)";
const MANIFEST_FILE: &str = "manifest.json";

#[derive(Debug, thiserror::Error)]
pub enum ShaderLoadError {
    #[error("translated shader manifest {0:?} is missing; {rerun}", rerun = RERUN_INSTALLER)]
    ManifestMissing(PathBuf),
    #[error("cannot read translated shader manifest {path:?}: {source}")]
    ManifestRead { path: PathBuf, source: io::Error },
    #[error("translated shader manifest {path:?} is not valid: {source}; {rerun}", rerun = RERUN_INSTALLER)]
    ManifestInvalid {
        path: PathBuf,
        source: serde_json::Error,
    },
    #[error(
        "translated shaders were produced by translator version {found}, \
         this engine expects {expected}; {rerun}",
        rerun = RERUN_INSTALLER
    )]
    VersionMismatch { expected: u32, found: u32 },
    #[error("translated shader {name:?} is listed twice in the manifest")]
    DuplicateName { name: String },
    #[error("translated shader {name:?} has file {file:?}, which is not a plain file name")]
    InvalidFileName { name: String, file: String },
    #[error("translated shader {name:?}: file {path:?} is missing; {rerun}", rerun = RERUN_INSTALLER)]
    ShaderFileMissing { name: String, path: PathBuf },
    #[error("translated shader {name:?}: cannot read {path:?}: {source}")]
    ShaderFileRead {
        name: String,
        path: PathBuf,
        source: io::Error,
    },
    #[error("translated shader {name:?} is not in the manifest; {rerun}", rerun = RERUN_INSTALLER)]
    UnknownShader { name: String },
    #[error(
        "translated shader {name:?} is a {found:?} shader in the installed shaders folder but the engine wrapper uses it as a {expected:?} shader; {rerun}",
        rerun = RERUN_INSTALLER
    )]
    StageMismatch {
        name: String,
        expected: ShaderStage,
        found: ShaderStage,
    },
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum ShaderStage {
    Vertex,
    Fragment,
    Compute,
}

#[derive(Debug, Deserialize)]
struct Manifest {
    translator_version: u32,
    shaders: Vec<ManifestEntry>,
}

pub const POSITION_SEMANTIC: &str = "SV_POSITION";

/// One signature element of a translated shader's inputs or outputs. A register shared by
/// several elements appears once per element, each with its own component mask.
#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
pub struct IoElement {
    pub register: u32,
    pub semantic: String,
    pub index: u32,
    pub mask: u8,
    pub system_value: u32,
}

impl IoElement {
    /// The `SV_Position` element, which the rasteriser writes (vertex outputs) or supplies
    /// (pixel inputs) instead of a varying.
    pub fn is_position(&self) -> bool {
        self.semantic.eq_ignore_ascii_case(POSITION_SEMANTIC)
    }
}

/// The register interface of a translated shader: what the engine needs to feed its inputs and
/// to connect its outputs without reading the WGSL.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct ShaderInterface {
    pub inputs: Vec<IoElement>,
    pub outputs: Vec<IoElement>,
}

#[derive(Debug, Deserialize)]
struct ManifestEntry {
    name: String,
    file: String,
    stage: ShaderStage,
    inputs: Vec<IoElement>,
    outputs: Vec<IoElement>,
}

/// A hand-written stand-in for a translated shader (name, stage, WGSL) used by tests.
#[cfg(test)]
pub type StubShader = (&'static str, ShaderStage, String);

#[derive(Debug)]
pub struct TranslatedShader {
    pub stage: ShaderStage,
    pub wgsl: String,
    pub interface: ShaderInterface,
}

#[derive(Debug, Default)]
pub struct TranslatedShaders {
    shaders: HashMap<String, TranslatedShader>,
}

impl TranslatedShaders {
    pub fn load(asset_root: &Path) -> Result<Self, ShaderLoadError> {
        let dir = asset_root.join(SHADERS_DIR);
        let manifest = read_manifest(&dir.join(MANIFEST_FILE))?;
        if manifest.translator_version != TRANSLATOR_VERSION {
            return Err(ShaderLoadError::VersionMismatch {
                expected: TRANSLATOR_VERSION,
                found: manifest.translator_version,
            });
        }
        let mut shaders = HashMap::with_capacity(manifest.shaders.len());
        for entry in manifest.shaders {
            if shaders.contains_key(&entry.name) {
                return Err(ShaderLoadError::DuplicateName { name: entry.name });
            }
            let wgsl = read_shader_file(&dir, &entry)?;
            shaders.insert(
                entry.name,
                TranslatedShader {
                    stage: entry.stage,
                    wgsl,
                    interface: ShaderInterface {
                        inputs: entry.inputs,
                        outputs: entry.outputs,
                    },
                },
            );
        }
        Ok(Self { shaders })
    }

    pub fn get(&self, name: &str) -> Result<&TranslatedShader, ShaderLoadError> {
        self.shaders
            .get(name)
            .ok_or_else(|| ShaderLoadError::UnknownShader {
                name: name.to_owned(),
            })
    }

    /// The WGSL of `name`, checked to be translated from a `stage` shader.
    pub fn wgsl(&self, name: &str, stage: ShaderStage) -> Result<&str, ShaderLoadError> {
        let shader = self.get(name)?;
        if shader.stage != stage {
            return Err(ShaderLoadError::StageMismatch {
                name: name.to_owned(),
                expected: stage,
                found: shader.stage,
            });
        }
        Ok(&shader.wgsl)
    }

    /// Names of every shader that starts with `prefix`, in no particular order.
    pub fn names_with_prefix<'a>(&'a self, prefix: &'a str) -> impl Iterator<Item = &'a str> {
        self.shaders
            .keys()
            .map(String::as_str)
            .filter(move |name| name.starts_with(prefix))
    }

    /// The register interface of `name`, checked to be translated from a `stage` shader.
    pub fn interface(
        &self,
        name: &str,
        stage: ShaderStage,
    ) -> Result<&ShaderInterface, ShaderLoadError> {
        self.wgsl(name, stage)?;
        Ok(&self.get(name)?.interface)
    }

    /// Replaces the interface of the stub `name`.
    #[cfg(test)]
    #[must_use]
    pub fn with_interface(mut self, name: &str, interface: ShaderInterface) -> Self {
        self.shaders
            .get_mut(name)
            .unwrap_or_else(|| panic!("stub {name} is not registered"))
            .interface = interface;
        self
    }

    #[cfg(test)]
    pub fn from_stubs(stubs: impl IntoIterator<Item = StubShader>) -> Self {
        Self::from_sources(
            stubs
                .into_iter()
                .collect::<Vec<_>>()
                .iter()
                .map(|(name, stage, wgsl)| (*name, *stage, wgsl.as_str())),
        )
    }

    #[cfg(test)]
    pub fn from_sources<'a>(
        sources: impl IntoIterator<Item = (&'a str, ShaderStage, &'a str)>,
    ) -> Self {
        let shaders = sources
            .into_iter()
            .map(|(name, stage, wgsl)| {
                (
                    name.to_owned(),
                    TranslatedShader {
                        stage,
                        wgsl: wgsl.to_owned(),
                        interface: ShaderInterface::default(),
                    },
                )
            })
            .collect();
        Self { shaders }
    }
}

fn read_manifest(path: &Path) -> Result<Manifest, ShaderLoadError> {
    let text = std::fs::read_to_string(path).map_err(|source| match source.kind() {
        io::ErrorKind::NotFound => ShaderLoadError::ManifestMissing(path.to_path_buf()),
        _ => ShaderLoadError::ManifestRead {
            path: path.to_path_buf(),
            source,
        },
    })?;
    serde_json::from_str(&text).map_err(|source| ShaderLoadError::ManifestInvalid {
        path: path.to_path_buf(),
        source,
    })
}

fn read_shader_file(dir: &Path, entry: &ManifestEntry) -> Result<String, ShaderLoadError> {
    let mut components = Path::new(&entry.file).components();
    let is_plain = matches!(
        (components.next(), components.next()),
        (Some(Component::Normal(_)), None)
    );
    if !is_plain {
        return Err(ShaderLoadError::InvalidFileName {
            name: entry.name.clone(),
            file: entry.file.clone(),
        });
    }
    let path = dir.join(&entry.file);
    std::fs::read_to_string(&path).map_err(|source| match source.kind() {
        io::ErrorKind::NotFound => ShaderLoadError::ShaderFileMissing {
            name: entry.name.clone(),
            path,
        },
        _ => ShaderLoadError::ShaderFileRead {
            name: entry.name.clone(),
            path,
            source,
        },
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::assets::layout::CACHE_DIR;
    use crate::installer_source;

    #[test]
    fn the_translator_version_is_the_one_the_installer_writes() {
        let installer =
            include_str!("../../../../installer/shader_translator/shader_translator_version.cs");
        assert_eq!(
            installer_source::constant(installer, "Value"),
            TRANSLATOR_VERSION.to_string()
        );
    }

    fn write_shaders_folder(root: &Path, manifest: &str, files: &[(&str, &str)]) {
        let dir = root.join(SHADERS_DIR);
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::write(dir.join(MANIFEST_FILE), manifest).unwrap();
        for (name, body) in files {
            std::fs::write(dir.join(name), body).unwrap();
        }
    }

    fn manifest(version: u32, entries: &str) -> String {
        format!(r#"{{"translator_version": {version}, "shaders": [{entries}]}}"#)
    }

    fn entry(name: &str, file: &str, stage: &str) -> String {
        format!(
            r#"{{"name": "{name}", "file": "{file}", "stage": "{stage}", "source_blob": 7, "source_sha256": "ab", "inputs": [], "outputs": []}}"#
        )
    }

    #[test]
    fn loads_every_listed_shader_by_name() {
        let root = tempfile::tempdir().unwrap();
        let entries = [
            entry("a", "a.wgsl", "fragment"),
            entry("b", "b.wgsl", "compute"),
        ]
        .join(",");
        write_shaders_folder(
            root.path(),
            &manifest(TRANSLATOR_VERSION, &entries),
            &[("a.wgsl", "fn a() {}"), ("b.wgsl", "fn b() {}")],
        );

        let shaders = TranslatedShaders::load(root.path()).unwrap();

        assert_eq!(
            shaders.wgsl("a", ShaderStage::Fragment).unwrap(),
            "fn a() {}"
        );
        assert_eq!(shaders.get("b").unwrap().stage, ShaderStage::Compute);
    }

    #[test]
    fn the_former_cache_location_is_not_read() {
        let root = tempfile::tempdir().unwrap();
        let former = root.path().join(CACHE_DIR).join(SHADERS_DIR);
        std::fs::create_dir_all(&former).unwrap();
        std::fs::write(former.join(MANIFEST_FILE), manifest(TRANSLATOR_VERSION, "")).unwrap();

        let error = TranslatedShaders::load(root.path()).unwrap_err();

        assert!(
            matches!(error, ShaderLoadError::ManifestMissing(_)),
            "{error}"
        );
    }

    #[test]
    fn errors_that_need_a_reinstall_name_the_installer_step_and_never_a_cache() {
        let errors = [
            ShaderLoadError::ManifestMissing(PathBuf::from("manifest.json")),
            ShaderLoadError::ManifestInvalid {
                path: PathBuf::from("manifest.json"),
                source: serde_json::from_str::<Manifest>("{").unwrap_err(),
            },
            ShaderLoadError::ShaderFileMissing {
                name: "a".to_owned(),
                path: PathBuf::from("a.wgsl"),
            },
            ShaderLoadError::UnknownShader {
                name: "a".to_owned(),
            },
            ShaderLoadError::VersionMismatch {
                expected: 2,
                found: 1,
            },
            ShaderLoadError::StageMismatch {
                name: "a".to_owned(),
                expected: ShaderStage::Fragment,
                found: ShaderStage::Vertex,
            },
        ];
        for error in errors {
            let message = error.to_string();
            assert!(message.contains("shader-translate"), "{message}");
            assert!(!message.to_lowercase().contains("cache"), "{message}");
        }
    }

    #[test]
    fn missing_manifest_is_reported() {
        let root = tempfile::tempdir().unwrap();
        let error = TranslatedShaders::load(root.path()).unwrap_err();
        assert!(
            matches!(error, ShaderLoadError::ManifestMissing(_)),
            "{error}"
        );
    }

    #[test]
    fn malformed_json_is_reported() {
        let root = tempfile::tempdir().unwrap();
        write_shaders_folder(root.path(), "{not json", &[]);
        let error = TranslatedShaders::load(root.path()).unwrap_err();
        assert!(
            matches!(error, ShaderLoadError::ManifestInvalid { .. }),
            "{error}"
        );
    }

    #[test]
    fn unknown_stage_is_reported_as_invalid_manifest() {
        let root = tempfile::tempdir().unwrap();
        let entries = entry("a", "a.wgsl", "geometry");
        write_shaders_folder(
            root.path(),
            &manifest(TRANSLATOR_VERSION, &entries),
            &[("a.wgsl", "")],
        );
        let error = TranslatedShaders::load(root.path()).unwrap_err();
        assert!(
            matches!(error, ShaderLoadError::ManifestInvalid { .. }),
            "{error}"
        );
    }

    #[test]
    fn version_mismatch_reports_both_versions() {
        let root = tempfile::tempdir().unwrap();
        write_shaders_folder(root.path(), &manifest(TRANSLATOR_VERSION + 1, ""), &[]);
        let error = TranslatedShaders::load(root.path()).unwrap_err();
        assert!(
            matches!(
                error,
                ShaderLoadError::VersionMismatch { expected, found }
                    if expected == TRANSLATOR_VERSION && found == TRANSLATOR_VERSION + 1
            ),
            "{error}"
        );
    }

    #[test]
    fn missing_shader_file_names_the_shader() {
        let root = tempfile::tempdir().unwrap();
        let entries = entry("water_ps", "water_ps.wgsl", "fragment");
        write_shaders_folder(root.path(), &manifest(TRANSLATOR_VERSION, &entries), &[]);
        let error = TranslatedShaders::load(root.path()).unwrap_err();
        assert!(
            matches!(&error, ShaderLoadError::ShaderFileMissing { name, .. } if name == "water_ps"),
            "{error}"
        );
    }

    #[test]
    fn duplicate_names_are_rejected() {
        let root = tempfile::tempdir().unwrap();
        let entries = [
            entry("a", "a.wgsl", "vertex"),
            entry("a", "b.wgsl", "vertex"),
        ]
        .join(",");
        write_shaders_folder(
            root.path(),
            &manifest(TRANSLATOR_VERSION, &entries),
            &[("a.wgsl", ""), ("b.wgsl", "")],
        );
        let error = TranslatedShaders::load(root.path()).unwrap_err();
        assert!(
            matches!(error, ShaderLoadError::DuplicateName { .. }),
            "{error}"
        );
    }

    #[test]
    fn file_names_escaping_the_shaders_dir_are_rejected() {
        let root = tempfile::tempdir().unwrap();
        for file in ["../x.wgsl", "sub/x.wgsl", ""] {
            let entries = entry("a", file, "vertex");
            write_shaders_folder(root.path(), &manifest(TRANSLATOR_VERSION, &entries), &[]);
            let error = TranslatedShaders::load(root.path()).unwrap_err();
            assert!(
                matches!(error, ShaderLoadError::InvalidFileName { .. }),
                "{file}: {error}"
            );
        }
    }

    #[test]
    fn lookup_of_unlisted_name_is_unknown_shader() {
        let shaders = TranslatedShaders::default();
        let error = shaders.wgsl("nope", ShaderStage::Vertex).unwrap_err();
        assert!(matches!(error, ShaderLoadError::UnknownShader { name } if name == "nope"));
    }

    const IO_ENTRY: &str = r#"{"name": "effect_ps_00000004", "file": "p.wgsl", "stage": "fragment",
        "source_blob": 7, "source_sha256": "ab",
        "inputs": [
            {"register": 0, "semantic": "SV_POSITION", "index": 0, "mask": 15, "system_value": 1},
            {"register": 1, "semantic": "TEXCOORD", "index": 0, "mask": 3, "system_value": 0},
            {"register": 1, "semantic": "TEXCOORD", "index": 1, "mask": 12, "system_value": 0}],
        "outputs": [{"register": 0, "semantic": "SV_Target", "index": 0, "mask": 15, "system_value": 0}]}"#;

    #[test]
    fn io_metadata_is_parsed_per_signature_element() {
        let root = tempfile::tempdir().unwrap();
        write_shaders_folder(
            root.path(),
            &manifest(TRANSLATOR_VERSION, IO_ENTRY),
            &[("p.wgsl", "")],
        );

        let shaders = TranslatedShaders::load(root.path()).unwrap();
        let interface = shaders
            .interface("effect_ps_00000004", ShaderStage::Fragment)
            .unwrap();

        assert_eq!(interface.inputs.len(), 3);
        assert!(interface.inputs[0].is_position());
        assert_eq!(interface.inputs[0].system_value, 1);
        assert_eq!(
            (
                interface.inputs[2].register,
                interface.inputs[2].index,
                interface.inputs[2].mask
            ),
            (1, 1, 12),
            "a register shared by two elements keeps one entry per element"
        );
        assert_eq!(interface.outputs[0].semantic, "SV_Target");
        assert!(!interface.outputs[0].is_position());
    }

    #[test]
    fn an_entry_without_io_metadata_is_an_invalid_manifest() {
        let root = tempfile::tempdir().unwrap();
        let entries = r#"{"name": "a", "file": "a.wgsl", "stage": "vertex"}"#;
        write_shaders_folder(
            root.path(),
            &manifest(TRANSLATOR_VERSION, entries),
            &[("a.wgsl", "")],
        );

        let error = TranslatedShaders::load(root.path()).unwrap_err();

        assert!(
            matches!(error, ShaderLoadError::ManifestInvalid { .. }),
            "{error}"
        );
    }

    #[test]
    fn the_interface_lookup_checks_the_stage() {
        let shaders = TranslatedShaders::from_sources([("a", ShaderStage::Vertex, "")]);
        assert!(shaders.interface("a", ShaderStage::Vertex).is_ok());
        assert!(matches!(
            shaders.interface("a", ShaderStage::Fragment),
            Err(ShaderLoadError::StageMismatch { .. })
        ));
    }

    #[test]
    fn names_are_listed_by_prefix() {
        let shaders = TranslatedShaders::from_sources([
            ("effect_vs_1", ShaderStage::Vertex, ""),
            ("effect_ps_1", ShaderStage::Fragment, ""),
            ("water_ps_depth", ShaderStage::Fragment, ""),
        ]);
        let mut names: Vec<_> = shaders.names_with_prefix("effect_").collect();
        names.sort_unstable();
        assert_eq!(names, ["effect_ps_1", "effect_vs_1"]);
    }

    #[test]
    fn lookup_with_the_wrong_stage_is_a_stage_mismatch() {
        let shaders = TranslatedShaders::from_sources([("a", ShaderStage::Vertex, "")]);
        let error = shaders.wgsl("a", ShaderStage::Fragment).unwrap_err();
        assert!(matches!(
            error,
            ShaderLoadError::StageMismatch {
                expected: ShaderStage::Fragment,
                found: ShaderStage::Vertex,
                ..
            }
        ));
    }
}
