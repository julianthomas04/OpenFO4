use glam::Vec3;

use super::role::TextureRole;
use crate::assets::types::effect_params::{EffectParams, EnvParams};
use crate::assets::types::material_name::MaterialName;
use crate::rendering::cube_array::CubeArray;
use crate::rendering::gpu_driven::{
    GpuMaterial, MaterialKey, MaterialRegistry, MaterialSlots, SurfaceParams, WHITE_SLOT,
};

/// Everything that decides a GPU material, borrowed from whichever decoded
/// material (a static submesh or a skinned part) it comes from.
pub struct MaterialSource<'a> {
    pub diffuse: Option<&'a str>,
    pub normal: Option<&'a str>,
    pub specular: Option<&'a str>,
    pub glow: Option<&'a str>,
    pub surface: &'a SurfaceParams,
    pub effect: Option<&'a EffectParams>,
    pub env: Option<&'a EnvParams>,
}

impl<'a> MaterialSource<'a> {
    pub fn of(name: &'a MaterialName) -> Self {
        Self {
            diffuse: name.diffuse_path.as_deref(),
            normal: name.normal_path.as_deref(),
            specular: name.specular_path.as_deref(),
            glow: name.glow_path.as_deref(),
            surface: &name.surface,
            effect: name.effect.as_ref(),
            env: name.env.as_ref(),
        }
    }

    /// `None` for a shape without a diffuse map that is not an effect shape:
    /// it draws the magenta fallback material. Effect shapes without a diffuse
    /// map draw white, tinted by their effect parameters.
    pub fn key(&self) -> Option<MaterialKey> {
        if self.diffuse.is_none() && self.effect.is_none() {
            return None;
        }
        Some(MaterialKey::new(
            self.diffuse,
            self.normal,
            self.specular,
            self.glow,
            self.surface,
            self.effect,
            self.env,
        ))
    }

    /// Every texture the material binds, by role: the worker's DDS pre-read and
    /// both acquisition paths iterate this, so their keys can't drift.
    pub fn texture_paths(&self) -> [(TextureRole, Option<&'a str>); 6] {
        [
            (TextureRole::Diffuse, self.diffuse),
            (TextureRole::Normal, self.normal),
            (TextureRole::Specular, self.specular),
            (TextureRole::Glow, self.glow),
            (
                TextureRole::EnvMask,
                self.env.and_then(|env| env.mask_path.as_deref()),
            ),
            (
                TextureRole::Palette,
                self.effect
                    .and_then(|effect| effect.palette.as_ref())
                    .map(|palette| palette.path.as_str()),
            ),
        ]
    }

    /// Slot of a role the material names no texture for. An emitting material
    /// without a glow map is lit by its emissive scalars alone, so it samples
    /// WHITE; an effect without a diffuse map is a flat colour, also WHITE;
    /// every other role uses its failed-load fallback.
    pub fn absent_slot(&self, role: TextureRole) -> u32 {
        if role == TextureRole::Diffuse && self.effect.is_some() {
            return WHITE_SLOT;
        }
        role.absent_slot(self.surface.emissive != Vec3::ZERO)
    }

    /// Resolves every texture role to a bindless slot: `acquire` admits a
    /// named texture, absent roles take their stand-in slot.
    pub fn resolve_slots(
        &self,
        mut acquire: impl FnMut(TextureRole, &str) -> u32,
    ) -> MaterialSlots {
        let [diffuse, normal, specular, glow, env_mask, palette] =
            self.texture_paths().map(|(role, path)| match path {
                Some(path) => acquire(role, path),
                None => self.absent_slot(role),
            });
        MaterialSlots {
            diffuse,
            normal,
            specular,
            glow,
            env_mask,
            palette,
        }
    }
}

/// Registers the material for `key` (from [`MaterialSource::key`]) and returns
/// its id; the environment cube is resolved to its slot on the way.
pub fn build_gpu_material(
    source: &MaterialSource,
    key: &MaterialKey,
    slots: MaterialSlots,
    cubes: &mut CubeArray,
    registry: &mut MaterialRegistry,
) -> u32 {
    let forced = cubes
        .forced_cube()
        .and_then(|cube| forced_environment(cube, source));
    let env = match &forced {
        Some(env) => Some(env),
        None => source.env,
    }
    .map(|env| (env, cubes.acquire(&env.cube_path)));
    registry.get_or_insert(
        key,
        GpuMaterial::new(slots, source.surface, source.effect, env),
    )
}

/// The environment a material reflects under the `OPENFO4_ENV_CUBE` override: its own mask
/// and scale with the forced cube, or, for an effect shape that authored no environment, the
/// forced cube alone. Materials that are neither keep their own (absent) environment.
fn forced_environment(cube: &str, source: &MaterialSource) -> Option<EnvParams> {
    if source.env.is_none() && source.effect.is_none() {
        return None;
    }
    Some(EnvParams {
        cube_path: cube.to_owned(),
        ..source.env.cloned().unwrap_or(EnvParams {
            cube_path: String::new(),
            mask_path: None,
            mask_scale: 1.0,
        })
    })
}

#[cfg(test)]
#[expect(clippy::float_cmp, reason = "tests compare copied values")]
mod tests {
    use super::*;

    fn name(text: &str) -> MaterialName {
        MaterialName::parse(text).unwrap()
    }

    fn key_of(text: &str) -> Option<MaterialKey> {
        MaterialSource::of(&name(text)).key()
    }

    #[test]
    fn bgem_without_diffuse_gets_a_key_and_plain_without_diffuse_does_not() {
        assert!(key_of("||blend|0.68|fx|bc:000000|bs:1|src:materials\\g.bgem").is_some());
        assert!(key_of("||blend|src:materials\\g.bgem").is_some());
        assert!(key_of("|").is_none());
        assert!(key_of("||blend").is_none());
    }

    #[test]
    fn key_distinguishes_env_cube_falloff_and_blend_mode() {
        let base = "a_d.dds||blend|fx|bc:FFFFFF|bs:1";
        let reference = key_of(base).unwrap();
        for variant in [
            format!("{base}|env:c1.dds"),
            format!("{base}|fo:1,0,0,1,0"),
            format!("{base}|bm:add"),
            format!("{base}|bm:mul"),
            format!("{base}|li:1"),
            format!("{base}|pal:c:p.dds"),
            "a_d.dds||blend|fx|bc:FFFFFF|bs:2".to_string(),
            "a_d.dds||blend|fx|bc:000000|bs:1".to_string(),
        ] {
            assert_ne!(key_of(&variant).unwrap(), reference, "{variant}");
        }
        assert_ne!(
            key_of(&format!("{base}|env:c1.dds")),
            key_of(&format!("{base}|env:c2.dds"))
        );
        assert_ne!(
            key_of(&format!("{base}|env:c1.dds|envs:0.5")),
            key_of(&format!("{base}|env:c1.dds|envs:0.6"))
        );
        assert_ne!(
            key_of(&format!("{base}|env:c1.dds|envk:m1.dds")),
            key_of(&format!("{base}|env:c1.dds|envk:m2.dds"))
        );
        assert_ne!(
            key_of(&format!("{base}|fo:1,0,0,1,0")),
            key_of(&format!("{base}|fo:1,0,0,1,1"))
        );
    }

    fn forced(text: &str) -> Option<EnvParams> {
        forced_environment("forced.dds", &MaterialSource::of(&name(text)))
    }

    #[test]
    fn forced_cube_replaces_the_authored_cube_and_keeps_mask_and_scale() {
        let env = forced("a_d.dds|a_n.dds|env:own.dds|envk:m.dds|envs:0.4").expect("env");
        assert_eq!(env.cube_path, "forced.dds");
        assert_eq!(env.mask_path.as_deref(), Some("textures\\m.dds"));
        assert_eq!(env.mask_scale, 0.4);
    }

    #[test]
    fn forced_cube_enables_reflection_on_effect_shapes_without_environment() {
        let env = forced("a_d.dds||blend|fx|bc:FFFFFF|bs:1").expect("env");
        assert_eq!(env.cube_path, "forced.dds");
        assert_eq!(env.mask_scale, 1.0);
    }

    #[test]
    fn forced_cube_leaves_plain_lit_materials_alone() {
        assert_eq!(forced("a_d.dds|a_n.dds"), None);
    }

    #[test]
    fn identical_names_share_a_key() {
        let text = "a_d.dds||blend|fx|bm:add|bc:FFFFFF|bs:1|env:c.dds";
        assert_eq!(key_of(text), key_of(text));
    }

    #[test]
    fn texture_paths_list_every_role_including_effect_ones() {
        let decoded = name("a_d.dds||blend|fx|bc:FFFFFF|bs:1|pal:c:p.dds|env:c.dds|envk:m.dds");
        let paths = MaterialSource::of(&decoded).texture_paths();
        let by_role = |role| paths.iter().find(|(r, _)| *r == role).unwrap().1;
        assert_eq!(by_role(TextureRole::Palette), Some("textures\\p.dds"));
        assert_eq!(by_role(TextureRole::EnvMask), Some("textures\\m.dds"));
        assert_eq!(by_role(TextureRole::Diffuse), Some("textures\\a_d.dds"));
    }

    #[test]
    fn effect_without_diffuse_samples_white_and_plain_missing_diffuse_the_fallback() {
        let effect = name("||blend|bc:FFFFFF");
        assert_eq!(
            MaterialSource::of(&effect).absent_slot(TextureRole::Diffuse),
            WHITE_SLOT
        );
        let plain = name("|");
        assert_ne!(
            MaterialSource::of(&plain).absent_slot(TextureRole::Diffuse),
            WHITE_SLOT
        );
    }

    #[test]
    fn resolve_slots_asks_for_named_textures_only() {
        let decoded = name("a_d.dds|a_n.dds|env:c.dds|envk:m.dds");
        let mut asked = Vec::new();
        let slots = MaterialSource::of(&decoded).resolve_slots(|role, path| {
            asked.push((role, path.to_string()));
            100 + asked.len() as u32
        });
        assert_eq!(asked.len(), 3);
        assert_eq!(slots.diffuse, 101);
        assert_eq!(slots.normal, 102);
        assert_eq!(slots.env_mask, 103);
        assert_eq!(slots.palette, WHITE_SLOT);
    }
}
