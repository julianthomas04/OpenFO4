//! Shader modules composed from repo-owned static parts and install-time translated parts.

use super::translated::{ShaderLoadError, ShaderStage, TranslatedShaders};

/// Module-scope directive every composed module with translated parts starts with. Directives
/// must precede all declarations, so it cannot live in a translated fragment or a wrapper.
pub const MODULE_PRELUDE: &str = "diagnostic(off, derivative_uniformity);\n";

#[derive(Debug, Clone, Copy)]
pub enum ShaderPart {
    /// Repo-owned WGSL; must not start with a module-scope directive.
    Static(&'static str),
    /// An install-time translated function, which the installed `shaders` folder must hold as `stage`.
    Translated {
        name: &'static str,
        stage: ShaderStage,
    },
}

impl ShaderPart {
    pub const fn vertex(name: &'static str) -> Self {
        Self::Translated {
            name,
            stage: ShaderStage::Vertex,
        }
    }

    pub const fn fragment(name: &'static str) -> Self {
        Self::Translated {
            name,
            stage: ShaderStage::Fragment,
        }
    }

    pub const fn compute(name: &'static str) -> Self {
        Self::Translated {
            name,
            stage: ShaderStage::Compute,
        }
    }
}

pub struct ComposedShaderSource {
    pub label: &'static str,
    pub parts: &'static [ShaderPart],
}

impl ComposedShaderSource {
    pub fn compose(&self, translated: &TranslatedShaders) -> Result<String, ShaderLoadError> {
        let mut source = String::new();
        if self
            .parts
            .iter()
            .any(|part| matches!(part, ShaderPart::Translated { .. }))
        {
            source.push_str(MODULE_PRELUDE);
        }
        for part in self.parts {
            match part {
                ShaderPart::Static(text) => source.push_str(text),
                ShaderPart::Translated { name, stage } => {
                    source.push_str(translated.wgsl(name, *stage)?);
                }
            }
        }
        Ok(source)
    }

    /// Composes and compiles for a device; a missing or mismatching part is an error.
    pub fn compile(
        &self,
        device: &wgpu::Device,
        translated: &TranslatedShaders,
    ) -> Result<wgpu::ShaderModule, ShaderLoadError> {
        let wgsl = self.compose(translated)?;
        Ok(device.create_shader_module(wgpu::ShaderModuleDescriptor {
            label: Some(self.label),
            source: wgpu::ShaderSource::Wgsl(wgsl.into()),
        }))
    }

    /// Compiles with the translated shaders loaded at startup. `startup::initialize` already
    /// composed and validated every registered module before any pipeline is built, so a
    /// failure here means the module is not in the registry (a programming error).
    ///
    /// wgpu validates the module once more inside `create_shader_module`; that second pass
    /// cannot be skipped through the safe API.
    pub fn compile_runtime(&self, device: &wgpu::Device) -> wgpu::ShaderModule {
        let translated = super::startup::translated()
            .expect("translated shaders are loaded at startup before any pipeline is built");
        self.compile(device, translated)
            .unwrap_or_else(|error| panic!("shader {:?}: {error}", self.label))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const PARTS: &[ShaderPart] = &[
        ShaderPart::Static("A;"),
        ShaderPart::fragment("t1"),
        ShaderPart::Static("B;"),
        ShaderPart::fragment("t2"),
    ];
    const STATIC_ONLY: ComposedShaderSource = ComposedShaderSource {
        label: "static_only",
        parts: &[ShaderPart::Static("A;")],
    };
    const SOURCE: ComposedShaderSource = ComposedShaderSource {
        label: "test",
        parts: PARTS,
    };

    fn translated() -> TranslatedShaders {
        TranslatedShaders::from_sources([
            ("t1", ShaderStage::Fragment, "T1;"),
            ("t2", ShaderStage::Fragment, "T2;"),
        ])
    }

    #[test]
    fn parts_are_concatenated_in_order_after_the_prelude() {
        assert_eq!(
            SOURCE.compose(&translated()).unwrap(),
            format!("{MODULE_PRELUDE}A;T1;B;T2;")
        );
    }

    #[test]
    fn the_prelude_appears_exactly_once_at_the_start() {
        let composed = SOURCE.compose(&translated()).unwrap();
        assert!(composed.starts_with(MODULE_PRELUDE));
        assert_eq!(composed.matches(MODULE_PRELUDE).count(), 1);
    }

    #[test]
    fn a_module_without_translated_parts_gets_no_prelude() {
        let composed = STATIC_ONLY.compose(&TranslatedShaders::default()).unwrap();
        assert_eq!(composed, "A;");
    }

    #[test]
    fn a_part_translated_from_another_stage_is_rejected() {
        let wrong_stage_shaders = TranslatedShaders::from_sources([
            ("t1", ShaderStage::Vertex, "T1;"),
            ("t2", ShaderStage::Fragment, "T2;"),
        ]);
        let error = SOURCE.compose(&wrong_stage_shaders).unwrap_err();
        assert!(matches!(
            error,
            ShaderLoadError::StageMismatch { name, expected: ShaderStage::Fragment, .. } if name == "t1"
        ));
    }

    #[test]
    fn missing_translated_part_names_the_shader() {
        let only_first = TranslatedShaders::from_sources([("t1", ShaderStage::Fragment, "T1;")]);
        let error = SOURCE.compose(&only_first).unwrap_err();
        assert!(matches!(error, ShaderLoadError::UnknownShader { name } if name == "t2"));
    }
}
