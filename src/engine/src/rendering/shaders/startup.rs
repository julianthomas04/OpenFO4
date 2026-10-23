//! Startup gate for composed shaders: load the installed translated shaders and naga-validate every module.

use std::path::Path;
use std::sync::{Arc, OnceLock};

use naga::valid::{Capabilities, ValidationFlags, Validator};

use super::composition::ComposedShaderSource;
use super::translated::{ShaderLoadError, TranslatedShaders};
use crate::rendering::effects;

#[derive(Debug, thiserror::Error)]
pub enum ShaderStartupError {
    #[error("fatal: {0}")]
    Load(#[from] ShaderLoadError),
    #[error("fatal: shader {label:?} failed WGSL parsing:\n{message}")]
    Parse {
        label: &'static str,
        message: String,
    },
    #[error("fatal: shader {label:?} failed WGSL validation:\n{message}")]
    Validation {
        label: &'static str,
        message: String,
    },
    #[error("fatal: {}", describe_effect_fault(.name, .fault))]
    EffectShader { name: String, fault: WgslFault },
    #[error("fatal: translated shaders were already initialized")]
    AlreadyInitialized,
}

fn describe_effect_fault(name: &str, fault: &WgslFault) -> String {
    match fault {
        WgslFault::Parse(message) => {
            format!("effect shader {name:?} failed WGSL parsing:\n{message}")
        }
        WgslFault::Validation(message) => {
            format!("effect shader {name:?} failed WGSL validation:\n{message}")
        }
    }
}

static TRANSLATED: OnceLock<TranslatedShaders> = OnceLock::new();
static EFFECT_VALIDATION: OnceLock<EffectValidation> = OnceLock::new();

/// Translated shaders loaded by [`initialize`]; `None` before it ran.
pub fn translated() -> Option<&'static TranslatedShaders> {
    TRANSLATED.get()
}

/// The background validation of the effect shaders that [`initialize`] started; `None` before it ran.
pub fn effect_validation() -> Option<&'static EffectValidation> {
    EFFECT_VALIDATION.get()
}

/// Loads the installed translated shaders and validates every module of `registry` against the
/// capabilities of a device with `device_features`. A missing `shaders` folder is fatal, also for
/// an empty registry. The effect family (over a thousand modules, composed per pair at run
/// time) is validated on a worker thread, see [`effect_validation`].
pub fn initialize(
    asset_root: &Path,
    registry: &[&ComposedShaderSource],
    device_features: wgpu::Features,
) -> Result<(), ShaderStartupError> {
    let translated = TranslatedShaders::load(asset_root)?;
    let capabilities = capabilities_of(device_features);
    validate_all(registry, &translated, capabilities)?;
    TRANSLATED
        .set(translated)
        .map_err(|_| ShaderStartupError::AlreadyInitialized)?;
    let installed = TRANSLATED
        .get()
        .expect("the translated shaders were stored just above");
    let _ = EFFECT_VALIDATION.set(spawn_effect_validation(installed, capabilities));
    Ok(())
}

/// Installs synthetic translated shaders for tests that build pipelines on a real device.
#[cfg(test)]
pub fn install_test_stubs() {
    let _ = TRANSLATED.set(super::composed::stub_translated_shaders());
}

/// The naga capabilities wgpu grants a device created with `features`, limited to the ones the
/// renderer's shaders can depend on. The baseline capabilities are downlevel flags every
/// desktop backend (D3D12, Vulkan, Metal) reports; wgpu maps them to naga capabilities itself.
pub fn capabilities_of(features: wgpu::Features) -> Capabilities {
    const FEATURE_CAPABILITIES: [(wgpu::Features, Capabilities); 4] = [
        (
            wgpu::Features::TEXTURE_BINDING_ARRAY,
            Capabilities::TEXTURE_AND_SAMPLER_BINDING_ARRAY,
        ),
        (
            wgpu::Features::SAMPLED_TEXTURE_AND_STORAGE_BUFFER_ARRAY_NON_UNIFORM_INDEXING,
            Capabilities::TEXTURE_AND_SAMPLER_BINDING_ARRAY_NON_UNIFORM_INDEXING
                .union(Capabilities::STORAGE_BUFFER_BINDING_ARRAY_NON_UNIFORM_INDEXING),
        ),
        (
            wgpu::Features::TEXTURE_FORMAT_16BIT_NORM,
            Capabilities::STORAGE_TEXTURE_16BIT_NORM_FORMATS,
        ),
        (
            wgpu::Features::SHADER_EARLY_DEPTH_TEST,
            Capabilities::EARLY_DEPTH_TEST,
        ),
    ];
    FEATURE_CAPABILITIES.iter().fold(
        Capabilities::MULTISAMPLED_SHADING
            | Capabilities::CUBE_ARRAY_TEXTURES
            | Capabilities::SHADER_FLOAT16_IN_FLOAT32,
        |capabilities, (feature, granted)| {
            if features.contains(*feature) {
                capabilities | *granted
            } else {
                capabilities
            }
        },
    )
}

pub fn validate_all(
    registry: &[&ComposedShaderSource],
    translated: &TranslatedShaders,
    capabilities: Capabilities,
) -> Result<(), ShaderStartupError> {
    for source in registry {
        let wgsl = source.compose(translated)?;
        check_wgsl(&wgsl, capabilities).map_err(|fault| match fault {
            WgslFault::Parse(message) => ShaderStartupError::Parse {
                label: source.label,
                message,
            },
            WgslFault::Validation(message) => ShaderStartupError::Validation {
                label: source.label,
                message,
            },
        })?;
    }
    Ok(())
}

/// Why a module's source was rejected, with the compiler's rendering of the problem.
#[derive(Debug)]
pub enum WgslFault {
    Parse(String),
    Validation(String),
}

/// Parses and validates one module's source.
pub fn check_wgsl(wgsl: &str, capabilities: Capabilities) -> Result<(), WgslFault> {
    let module = naga::front::wgsl::parse_str(wgsl)
        .map_err(|error| WgslFault::Parse(error.emit_to_string(wgsl)))?;
    Validator::new(ValidationFlags::all(), capabilities)
        .validate(&module)
        .map_err(|error| WgslFault::Validation(error.emit_to_string(wgsl)))?;
    Ok(())
}

/// Outcome of the background validation of the effect shader family.
#[derive(Debug)]
pub enum EffectValidationStatus<'a> {
    Pending,
    Passed { modules: usize },
    Failed(&'a ShaderStartupError),
}

/// A running (or finished) validation of every translated effect shader. The result is read
/// without blocking through [`Self::status`], so a frame loop can poll it.
#[derive(Debug, Clone)]
pub struct EffectValidation {
    result: Arc<OnceLock<Result<usize, ShaderStartupError>>>,
}

impl EffectValidation {
    /// The error of a finished validation that failed; `None` while it runs or after it passed.
    pub fn failure(&self) -> Option<&ShaderStartupError> {
        match self.status() {
            EffectValidationStatus::Failed(error) => Some(error),
            EffectValidationStatus::Pending | EffectValidationStatus::Passed { .. } => None,
        }
    }

    pub fn status(&self) -> EffectValidationStatus<'_> {
        match self.result.get() {
            None => EffectValidationStatus::Pending,
            Some(Ok(modules)) => EffectValidationStatus::Passed { modules: *modules },
            Some(Err(error)) => EffectValidationStatus::Failed(error),
        }
    }
}

/// Starts validating every effect vertex and pixel shader on a worker thread. A failure is
/// logged at once and stays readable through the returned handle.
pub fn spawn_effect_validation(
    translated: &'static TranslatedShaders,
    capabilities: Capabilities,
) -> EffectValidation {
    let result = Arc::new(OnceLock::new());
    let outcome = Arc::clone(&result);
    std::thread::Builder::new()
        .name("effect-shader-validation".to_owned())
        .spawn(move || {
            let started = std::time::Instant::now();
            let validated = validate_effect_shaders(translated, capabilities);
            match &validated {
                Ok(modules) => tracing::info!(
                    "validated {modules} effect shader modules in {:?}",
                    started.elapsed()
                ),
                Err(error) => tracing::error!("{error}"),
            }
            let _ = outcome.set(validated);
        })
        .expect("the effect shader validation thread can be spawned");
    EffectValidation { result }
}

/// Validates every `effect_vs_*` and `effect_ps_*` shader on its own (wrapper plus that one
/// function, no pair glue), returning how many modules passed. The first failure is fatal.
pub fn validate_effect_shaders(
    translated: &TranslatedShaders,
    capabilities: Capabilities,
) -> Result<usize, ShaderStartupError> {
    let mut names: Vec<&str> = translated
        .names_with_prefix(effects::shader::EFFECT_SHADER_PREFIX)
        .collect();
    names.sort_unstable();
    for name in &names {
        let wgsl = effects::shader::compose_standalone(name, translated)?;
        check_wgsl(&wgsl, capabilities).map_err(|fault| ShaderStartupError::EffectShader {
            name: (*name).to_owned(),
            fault,
        })?;
    }
    Ok(names.len())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::assets::layout::SHADERS_DIR;
    use crate::rendering::shaders::composition::ShaderPart;
    use crate::rendering::shaders::translated::{ShaderStage, TRANSLATOR_VERSION};

    const EARLY_DEPTH_FRAGMENT: &str = "@fragment @early_depth_test(force) fn early() {}\n";
    const EARLY_DEPTH_PARTS: &[ShaderPart] = &[ShaderPart::Static(EARLY_DEPTH_FRAGMENT)];
    const EARLY_DEPTH_MODULE: ComposedShaderSource = ComposedShaderSource {
        label: "early_depth_module",
        parts: EARLY_DEPTH_PARTS,
    };

    const WRAPPER: &str = "@fragment fn main() -> @location(0) vec4<f32> { return helper(); }\n";
    const GOOD_HELPER: &str = "fn helper() -> vec4<f32> { return vec4<f32>(1.0); }\n";
    const BAD_HELPER: &str = "fn helper() -> vec4<f32> { var v: vec4<f32>; }
";

    const PARTS: &[ShaderPart] = &[ShaderPart::Static(WRAPPER), ShaderPart::fragment("helper")];
    const MODULE: ComposedShaderSource = ComposedShaderSource {
        label: "test_module",
        parts: PARTS,
    };

    fn translated(helper: &str) -> TranslatedShaders {
        TranslatedShaders::from_sources([("helper", ShaderStage::Fragment, helper)])
    }

    #[test]
    fn valid_module_passes() {
        validate_all(&[&MODULE], &translated(GOOD_HELPER), Capabilities::empty()).unwrap();
    }

    #[test]
    fn syntax_error_is_fatal_and_names_the_shader() {
        let error = validate_all(
            &[&MODULE],
            &translated("fn helper( {"),
            Capabilities::empty(),
        )
        .unwrap_err();
        assert!(matches!(
            error,
            ShaderStartupError::Parse {
                label: "test_module",
                ..
            }
        ));
        assert!(
            error
                .to_string()
                .starts_with("fatal: shader \"test_module\"")
        );
    }

    #[test]
    fn type_error_is_fatal_validation_error() {
        let error =
            validate_all(&[&MODULE], &translated(BAD_HELPER), Capabilities::empty()).unwrap_err();
        assert!(matches!(
            error,
            ShaderStartupError::Validation {
                label: "test_module",
                ..
            }
        ));
    }

    #[test]
    fn validation_uses_the_capabilities_of_the_device() {
        let without = capabilities_of(wgpu::Features::empty());
        let with = capabilities_of(wgpu::Features::SHADER_EARLY_DEPTH_TEST);
        let translated = TranslatedShaders::default();

        let rejected = validate_all(&[&EARLY_DEPTH_MODULE], &translated, without).unwrap_err();
        validate_all(&[&EARLY_DEPTH_MODULE], &translated, with).unwrap();

        assert!(matches!(rejected, ShaderStartupError::Validation { .. }));
    }

    #[test]
    fn the_required_device_features_cover_what_the_shaders_use() {
        let capabilities = capabilities_of(crate::rendering::device::REQUIRED_FEATURES);
        assert!(capabilities.contains(Capabilities::TEXTURE_AND_SAMPLER_BINDING_ARRAY));
        assert!(capabilities.contains(Capabilities::EARLY_DEPTH_TEST));
        assert!(capabilities.contains(Capabilities::STORAGE_TEXTURE_16BIT_NORM_FORMATS));
    }

    #[test]
    fn the_validation_capabilities_match_the_installer_test_harness() {
        const INSTALLER_NAGA_CAPABILITIES: &str = "TEXTURE_AND_SAMPLER_BINDING_ARRAY,STORAGE_TEXTURE_16BIT_NORM_FORMATS,EARLY_DEPTH_TEST,MULTISAMPLED_SHADING,CUBE_ARRAY_TEXTURES,SHADER_FLOAT16_IN_FLOAT32,TEXTURE_AND_SAMPLER_BINDING_ARRAY_NON_UNIFORM_INDEXING,STORAGE_BUFFER_BINDING_ARRAY_NON_UNIFORM_INDEXING";
        let installer = INSTALLER_NAGA_CAPABILITIES
            .split(',')
            .map(|name| Capabilities::from_name(name).expect("a naga capability name"))
            .fold(Capabilities::empty(), |all, one| all | one);

        assert_eq!(
            capabilities_of(crate::rendering::device::REQUIRED_FEATURES),
            installer
        );
    }

    #[test]
    fn half_float_unpacking_validates_with_the_device_capabilities() {
        const UNPACK: &str =
            "fn half(bits: u32) -> f32 { return unpack2x16float(bits & 0xffffu).x; }
";
        let capabilities = capabilities_of(crate::rendering::device::REQUIRED_FEATURES);

        check_wgsl(UNPACK, capabilities).unwrap();
        assert!(matches!(
            check_wgsl(UNPACK, Capabilities::empty()),
            Err(WgslFault::Validation(_))
        ));
    }

    #[test]
    fn missing_translated_shader_is_fatal() {
        let error = validate_all(
            &[&MODULE],
            &TranslatedShaders::default(),
            Capabilities::empty(),
        )
        .unwrap_err();
        assert!(matches!(
            error,
            ShaderStartupError::Load(ShaderLoadError::UnknownShader { .. })
        ));
    }

    #[test]
    fn every_effect_shader_is_validated_on_its_own() {
        let translated = effects::shader::stubs::translated();
        let capabilities = capabilities_of(crate::rendering::device::REQUIRED_FEATURES);

        let modules = validate_effect_shaders(&translated, capabilities).unwrap();

        assert_eq!(modules, 6);
    }

    #[test]
    fn an_invalid_effect_shader_is_fatal_and_named() {
        let translated = effects::shader::stubs::translated_with_mesh_vertex_wgsl(BAD_HELPER);
        let capabilities = capabilities_of(crate::rendering::device::REQUIRED_FEATURES);

        let error = validate_effect_shaders(&translated, capabilities).unwrap_err();

        assert!(
            matches!(
                &error,
                ShaderStartupError::EffectShader { name, fault: WgslFault::Validation(_) }
                    if name == effects::shader::stubs::MESH_VS
            ),
            "{error}"
        );
        assert!(
            error
                .to_string()
                .starts_with("fatal: effect shader \"effect_vs_00000005\"")
        );
    }

    #[test]
    fn the_background_validation_reports_through_its_handle() {
        let capabilities = capabilities_of(crate::rendering::device::REQUIRED_FEATURES);
        let good: &'static TranslatedShaders =
            Box::leak(Box::new(effects::shader::stubs::translated()));
        let bad: &'static TranslatedShaders = Box::leak(Box::new(
            effects::shader::stubs::translated_with_mesh_vertex_wgsl("fn broken( {"),
        ));

        let passed = spawn_effect_validation(good, capabilities);
        let failed = spawn_effect_validation(bad, capabilities);

        let wait = |validation: &EffectValidation| {
            let deadline = std::time::Instant::now() + std::time::Duration::from_secs(30);
            while matches!(validation.status(), EffectValidationStatus::Pending) {
                assert!(
                    std::time::Instant::now() < deadline,
                    "validation did not finish"
                );
                std::thread::yield_now();
            }
        };
        wait(&passed);
        wait(&failed);
        assert!(matches!(
            passed.status(),
            EffectValidationStatus::Passed { modules: 6 }
        ));
        assert!(matches!(
            failed.status(),
            EffectValidationStatus::Failed(ShaderStartupError::EffectShader {
                fault: WgslFault::Parse(_),
                ..
            })
        ));
        assert!(passed.failure().is_none());
        let error = failed
            .failure()
            .expect("the broken shader fails validation");
        assert!(
            error.to_string().starts_with("fatal: effect shader"),
            "{error}"
        );
    }

    #[test]
    fn nonempty_registry_without_installed_shaders_is_fatal() {
        let root = tempfile::tempdir().unwrap();
        let error = initialize(root.path(), &[&MODULE], wgpu::Features::empty()).unwrap_err();
        assert!(matches!(
            error,
            ShaderStartupError::Load(ShaderLoadError::ManifestMissing(_))
        ));
    }

    #[test]
    fn nonempty_registry_validates_the_loaded_shaders() {
        let root = tempfile::tempdir().unwrap();
        let dir = root.path().join(SHADERS_DIR);
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::write(dir.join("helper.wgsl"), BAD_HELPER).unwrap();
        std::fs::write(
            dir.join("manifest.json"),
            format!(
                r#"{{"translator_version": {TRANSLATOR_VERSION}, "shaders": [
                {{"name": "helper", "file": "helper.wgsl", "stage": "fragment",
                 "inputs": [], "outputs": []}}]}}"#
            ),
        )
        .unwrap();

        let error = initialize(root.path(), &[&MODULE], wgpu::Features::empty()).unwrap_err();

        assert!(matches!(error, ShaderStartupError::Validation { .. }));
    }
}
