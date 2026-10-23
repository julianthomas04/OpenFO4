use crate::assets::types::effect_params::EffectBlend;
use crate::assets::types::material_name::MaterialName;
use crate::rendering::cube_array::CubeArray;
use crate::rendering::gpu_driven::{
    FALLBACK_MATERIAL_ID, MaterialRegistry, TextureArray, TextureHandle,
};
use crate::rendering::texture_streaming::material::{MaterialSource, build_gpu_material};
use crate::rendering::texture_streaming::residency::TextureResidency;
use crate::rendering::texture_streaming::role::{Admission, TextureRole, admit_texture};
use crate::utils::warn_once;

/// The GPU services a character needs to turn a material name into a material id.
pub struct CharacterGpuContext<'a> {
    pub device: &'a wgpu::Device,
    pub queue: &'a wgpu::Queue,
    pub texture_array: &'a mut TextureArray,
    pub residency: &'a mut TextureResidency,
    pub material_registry: &'a mut MaterialRegistry,
    pub cube_array: &'a mut CubeArray,
}

impl CharacterGpuContext<'_> {
    pub fn release_textures(&mut self, references: &[TextureHandle]) {
        for &handle in references {
            if self.texture_array.release(handle) {
                self.residency.forget(handle);
            }
        }
    }
}

/// A per-placement material swap: parts whose source material is `original`
/// draw with `replacement` (same name grammar as the GLB's).
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct MaterialOverride {
    pub original: String,
    pub replacement: String,
}

/// How a mesh's parts are turned into draws.
#[derive(Clone, Copy)]
pub enum PartPolicy<'a> {
    /// Every part draws with its own material.
    Creature,
    /// A placed prop follows the static path: material swaps apply, helper and
    /// water shapes are not drawn, effect shapes are.
    Prop(&'a [MaterialOverride]),
}

impl<'a> PartPolicy<'a> {
    /// Whether a part draws only while the debug "Show FX" toggle is on. A
    /// placed prop follows the static path's rule for skinned shapes; creatures always draw.
    pub fn hides_part_by_default(self, name: Option<&str>) -> bool {
        matches!(self, Self::Prop(_))
            && name
                .and_then(|name| MaterialName::parse(name).ok())
                .is_some_and(|original| original.is_hidden_skinned_effect_shape())
    }

    /// The material name a part draws with, `None` when the part is not drawn.
    /// Shader class is judged on the original name, as a swap never changes it.
    pub fn material_name(self, name: Option<&'a str>) -> Option<Option<&'a str>> {
        let Self::Prop(overrides) = self else {
            return Some(name);
        };
        let Some(Ok(original)) = name.map(MaterialName::parse) else {
            return Some(name);
        };
        if original.is_helper_shape() || original.is_water_shape() {
            return None;
        }
        let swapped = original.source_material.as_deref().and_then(|source| {
            overrides
                .iter()
                .find(|candidate| candidate.original == source)
        });
        Some(swapped.map_or(name, |swap| Some(swap.replacement.as_str())))
    }
}

/// Which pipeline family draws a surface, derived like the static buckets.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum SurfaceKind {
    Opaque,
    AlphaTest,
    Blend,
    Multiply,
}

impl SurfaceKind {
    /// Drawn in the translucent pass, after the opaque world and the water.
    pub fn is_blended(self) -> bool {
        matches!(self, Self::Blend | Self::Multiply)
    }
}

/// The pipeline family a decoded material draws with.
pub fn surface_kind(name: &MaterialName) -> SurfaceKind {
    let multiplies = name
        .effect
        .as_ref()
        .is_some_and(|effect| effect.blend == EffectBlend::Multiply);
    if name.is_blend && multiplies {
        SurfaceKind::Multiply
    } else if name.is_blend {
        SurfaceKind::Blend
    } else if name.alpha_cutoff > 0.0 {
        SurfaceKind::AlphaTest
    } else {
        SurfaceKind::Opaque
    }
}

pub struct ResolvedMaterial {
    pub material_id: u32,
    pub kind: SurfaceKind,
    /// Effect-shader material: never casts a shadow, whatever its kind.
    pub is_effect: bool,
    pub two_sided: bool,
    pub alpha_cutoff: f32,
    pub alpha_mult: f32,
    /// One array reference per texture the material names; the rig releases them.
    pub texture_refs: Vec<TextureHandle>,
}

/// Mirrors the static meshes' material path: decode the name, admit its
/// textures into the bindless array, register the GPU material. A name that is
/// absent or fails to decode draws the magenta fallback material. Shared by
/// every GPU-side material consumer outside the placed-object path (characters,
/// skinned props, grass).
pub fn resolve_material(name: Option<&str>, gpu: &mut CharacterGpuContext) -> ResolvedMaterial {
    let decoded = match name.map(MaterialName::parse) {
        Some(Ok(decoded)) => decoded,
        Some(Err(error)) => {
            warn_once(&format!("material-name:{name:?}"), || {
                format!("material name {name:?} is invalid: {error}; using the fallback material")
            });
            MaterialName::untextured()
        }
        None => MaterialName::untextured(),
    };

    let mut texture_refs = Vec::new();
    let source = MaterialSource::of(&decoded);
    let material_id = match source.key() {
        Some(key) => {
            let slots = source
                .resolve_slots(|role, path| acquire_texture(path, role, gpu, &mut texture_refs));
            if decoded.alpha_cutoff > 0.0 && source.diffuse.is_some() {
                gpu.texture_array.mark_alpha_tested(slots.diffuse);
            }
            build_gpu_material(&source, &key, slots, gpu.cube_array, gpu.material_registry)
        }
        None => FALLBACK_MATERIAL_ID,
    };

    ResolvedMaterial {
        material_id,
        kind: surface_kind(&decoded),
        is_effect: decoded.effect.is_some(),
        two_sided: decoded.two_sided,
        alpha_cutoff: decoded.alpha_cutoff,
        alpha_mult: decoded.alpha_mult,
        texture_refs,
    }
}

/// Textures that cannot be admitted fall back to the role's stand-in slot, as
/// in the static path; the cause is logged once per texture. Characters are
/// built synchronously on the render thread (debug spawner), so a spawn can
/// hitch while textures upload.
fn acquire_texture(
    path: &str,
    role: TextureRole,
    gpu: &mut CharacterGpuContext,
    references: &mut Vec<TextureHandle>,
) -> u32 {
    let fallback_slot = role.fallback_slot();
    match admit_texture(
        gpu.texture_array,
        gpu.residency,
        gpu.device,
        gpu.queue,
        path,
        role.is_srgb(),
        fallback_slot,
        None,
        false,
    ) {
        Admission::Admitted { handle, .. } => {
            references.push(handle);
            handle.slot
        }
        Admission::Exhausted => {
            warn_once(&format!("material-texture-exhausted:{path}"), || {
                format!("texture array exhausted, '{path}' uses the fallback slot")
            });
            fallback_slot
        }
        Admission::Failed => fallback_slot,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn kind_of(name: &str) -> SurfaceKind {
        surface_kind(&MaterialName::parse(name).expect("valid name"))
    }

    #[test]
    fn plain_materials_are_opaque() {
        assert_eq!(kind_of("a_d.dds|a_n.dds"), SurfaceKind::Opaque);
    }

    #[test]
    fn an_alpha_cutoff_selects_alpha_test() {
        assert_eq!(kind_of("a_d.dds|a_n.dds|test:128"), SurfaceKind::AlphaTest);
    }

    #[test]
    fn blend_selects_the_blend_kind_for_alpha_and_additive_modes() {
        assert_eq!(kind_of("a_d.dds|a_n.dds|blend"), SurfaceKind::Blend);
        assert_eq!(
            kind_of("a_d.dds||blend|bm:add|bc:FFFFFF|bs:1"),
            SurfaceKind::Blend
        );
    }

    #[test]
    fn multiplicative_blend_selects_the_multiply_kind() {
        assert_eq!(
            kind_of("a_d.dds||blend|bm:mul|bc:FFFFFF|bs:1"),
            SurfaceKind::Multiply
        );
    }

    #[test]
    fn multiply_mode_without_blend_stays_opaque() {
        assert_eq!(
            kind_of("a_d.dds||bm:mul|bc:FFFFFF|bs:1"),
            SurfaceKind::Opaque
        );
    }
}
