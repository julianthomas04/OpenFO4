//! Which water shader permutation, and which extra passes, a water surface
//! draws with: the render-pass selection for a water shader property.
//!
//! The descriptor is a bit set derived from the property's flags; the shader permutations
//! the engine has are picked from its bits ([`Variant`]), and the bits it cannot honour are
//! reported in [`SurfacePasses::unsupported`] instead of being silently ignored.

use crate::assets::types::water_shader::water_flags;

/// Descriptor bits.
pub mod descriptor {
    pub const VERTEX_COLOUR: u32 = 0x1;
    pub const NORMAL_TEXCOORD: u32 = 0x2;
    pub const REFLECTIONS: u32 = 0x4;
    pub const REFRACTIONS: u32 = 0x8;
    pub const DEPTH: u32 = 0x10;
    /// Interior permutation: removes the direct sun terms (sparkle and
    /// specular), keeps the fog's sun in-scatter.
    pub const INTERIOR: u32 = 0x20;
    pub const WADING: u32 = 0x40;
    pub const VERTEX_ALPHA_DEPTH: u32 = 0x80;
    pub const SSR: u32 = 0x200;
    pub const UNDERWATER: u32 = 0x1800;
    /// The underwater pixel shader family (descriptor bit 0x1000).
    pub const UNDERWATER_SHADER: u32 = 0x1000;
    pub const LOD: u32 = 0x10000;
    /// Depth-fog colour pass; with [`UNDERWATER_SHADER`] it is the constant
    /// zero pass instead (`0x19xxx`: output `(0, 0, 0, 0)`).
    pub const DEPTH_FOG_PASS: u32 = 0x18010;
    /// Constant-colour pass writing the tag 2 into the water mask.
    pub const MASK_SURFACE_PASS: u32 = 0x12000;
    /// Constant-colour pass writing the tag 3 into the water mask (wading).
    pub const MASK_WADING_PASS: u32 = 0x14040;
}

/// Descriptor bits the engine has no shader for. A surface whose descriptor
/// carries one still draws, with the nearest permutation, and says so. The
/// vertex-alpha depth bit is only supported with the underwater shader.
const UNSUPPORTED_BITS: u32 = descriptor::VERTEX_ALPHA_DEPTH;

/// Per-frame facts the pass selection reads besides the property itself.
#[derive(Debug, Copy, Clone, Default, PartialEq, Eq)]
pub struct PassContext {
    /// The surface is in an interior cell (assumed to be the flag that selects the interior
    /// permutation).
    pub interior: bool,
    /// The camera is under water: the property's own underwater bit is
    /// overwritten with this every frame.
    pub underwater: bool,
    /// The runtime toggle and the WATR's Screen Space Reflections flag are both
    /// set; the descriptor then carries the SSR bit (LOD surfaces as well).
    pub screen_space_reflections: bool,
}

/// The mask value a surface writes (the `0x12xxx` / `0x14xxx` constant passes).
#[derive(Debug, Copy, Clone, PartialEq, Eq)]
pub enum MaskTag {
    /// Alpha `0.0078432` (2/255), the regular surface pass.
    Surface,
    /// Alpha `0.0117648` (3/255), the wading pass.
    Wading,
}

impl MaskTag {
    pub const fn value(self) -> u8 {
        match self {
            Self::Surface => 2,
            Self::Wading => 3,
        }
    }
}

/// The shader permutations of `water_wrapper.wgsl`: which pixel shader family and
/// whether the interior (no direct sun) code is selected.
#[derive(Debug, Copy, Clone, PartialEq, Eq)]
pub struct Variant {
    /// The depth shader (depth un-projection, `t7`), else refraction only.
    pub depth: bool,
    /// The depth shader's interior permutation (descriptor bit `0x20`). The refraction-only
    /// shader has no interior permutation, so this is never set without `depth`;
    /// its interior surfaces get the no-sun terms zeroed through the material registers.
    pub interior: bool,
    /// The LOD water shader (descriptor `0x10000`): no refraction, no depth.
    pub lod: bool,
    /// The LOD water shader with the two SSR taps (descriptor `0x10204`).
    pub ssr: bool,
    /// The camera-under-water shader (descriptor bit `0x1000`): its four descriptors
    /// (`0x180c` / `0x184e` / `0x1a0c` / `0x1a4e`) merged like the above-water ones
    /// (ripple and SSR taps read stand-ins when unused).
    pub underwater: bool,
}

impl Variant {
    pub const COUNT: usize = 6;

    pub const fn index(self) -> usize {
        if self.underwater {
            return 5;
        }
        if self.lod {
            return 3 + self.ssr as usize;
        }
        if self.depth {
            return 1 + self.interior as usize;
        }
        0
    }

    pub const ALL: [Self; Self::COUNT] = [
        Self::refraction_only(),
        Self::depth(false),
        Self::depth(true),
        Self::lod(false),
        Self::lod(true),
        Self::underwater(),
    ];

    const fn refraction_only() -> Self {
        Self {
            depth: false,
            interior: false,
            lod: false,
            ssr: false,
            underwater: false,
        }
    }

    const fn depth(interior: bool) -> Self {
        Self {
            depth: true,
            interior,
            lod: false,
            ssr: false,
            underwater: false,
        }
    }

    const fn lod(ssr: bool) -> Self {
        Self {
            depth: false,
            interior: false,
            lod: true,
            ssr,
            underwater: false,
        }
    }

    const fn underwater() -> Self {
        Self {
            depth: false,
            interior: false,
            lod: false,
            ssr: false,
            underwater: true,
        }
    }
}

/// Everything the pass selection yields for one water surface.
#[derive(Debug, Copy, Clone, PartialEq, Eq)]
pub struct SurfacePasses {
    /// The main pass descriptor `d`.
    pub main_descriptor: u32,
    pub variant: Variant,
    /// The depth-fog colour pass (descriptor `0x18010 | ...`). Under water the
    /// slot is the constant-zero pass `0x19010`, which changes nothing and is
    /// not drawn.
    pub depth_fog: Option<u32>,
    /// The constant-colour pass tagging the refraction mask; LOD water has none
    /// (the `0x12000` / `0x14040` passes are added for non-LOD only).
    pub mask_tag: Option<MaskTag>,
    /// Descriptor bits set that no permutation honours.
    pub unsupported: u32,
}

impl SurfacePasses {
    /// The surface reads the wading displacement field (`t2`): the wading
    /// descriptor on a shader that has the ripple lookup.
    pub fn reads_displacement(&self) -> bool {
        self.main_descriptor & descriptor::WADING != 0
            && (self.variant.depth || self.variant.underwater)
    }
}

/// The pass selection for a non-LOD or LOD surface with the property flags
/// `flags` ([`water_flags`]).
pub fn select(flags: u32, context: PassContext) -> SurfacePasses {
    use descriptor as bit;
    let has = |flag: u32| flags & flag != 0;
    let wading = has(water_flags::DISPLACEMENT);
    let lod = has(water_flags::LOD);

    if lod {
        return lod_passes(
            has(water_flags::REFLECTIONS),
            context.screen_space_reflections,
        );
    }

    let mut main = 0;
    if has(water_flags::VERTEX_UV | water_flags::VERTEX_ALPHA_DEPTH) {
        main |= bit::VERTEX_COLOUR;
    }
    if wading {
        main |= bit::NORMAL_TEXCOORD;
    }
    if has(water_flags::REFLECTIONS) {
        main |= bit::REFLECTIONS;
    }
    if has(water_flags::REFRACTIONS) {
        main |= bit::REFRACTIONS;
    }
    if context.interior {
        main |= bit::INTERIOR;
    }
    if wading {
        main |= bit::WADING;
    }
    if has(water_flags::VERTEX_ALPHA_DEPTH) {
        main |= bit::VERTEX_ALPHA_DEPTH;
    }
    if context.underwater {
        main |= bit::UNDERWATER;
    } else if has(water_flags::DEPTH) {
        main |= bit::DEPTH;
    }
    if context.screen_space_reflections {
        main |= bit::SSR;
    }
    let depth_fog = (!wading && has(water_flags::DEPTH) && !context.underwater)
        .then_some(bit::DEPTH_FOG_PASS | (main & bit::VERTEX_COLOUR));

    let underwater = main & bit::UNDERWATER_SHADER != 0;
    let depth =
        main & bit::DEPTH != 0 && main & (bit::VERTEX_ALPHA_DEPTH | bit::UNDERWATER_SHADER) == 0;
    let variant = Variant {
        depth,
        interior: depth && main & bit::INTERIOR != 0,
        lod: false,
        ssr: false,
        underwater,
    };
    // Only the depth and the underwater shader carry the ripple lookup; a
    // wading surface on the refraction-only shader has no displacement
    // permutation here.
    let missing_ripple = if wading && !variant.depth && !underwater {
        bit::WADING
    } else {
        0
    };

    SurfacePasses {
        main_descriptor: main,
        variant,
        depth_fog,
        mask_tag: Some(if wading {
            MaskTag::Wading
        } else {
            MaskTag::Surface
        }),
        unsupported: (if underwater {
            0
        } else {
            main & UNSUPPORTED_BITS
        }) | missing_ripple,
    }
}

/// The property flags of the displacement quad added for a water object
/// with flags `source`: displacement and depth on, reflections, refractions and
/// cubemap inherited.
pub fn displacement_quad_flags(source: u32) -> u32 {
    water_flags::DISPLACEMENT
        | water_flags::DEPTH
        | source & (water_flags::REFLECTIONS | water_flags::REFRACTIONS | water_flags::CUBEMAP)
}

/// The LOD branch of the pass selection: `0x10000`, plus `0x4` with the reflections flag, plus
/// `0x200` (`0x204` together) when SSR is on as well. Interior, underwater, depth and wading bits
/// are never added, and there is no depth-fog or mask pass. The `0x400` (clip ellipsoid) bit, set
/// while the global clip ellipsoid exists, selects a vertex shader variant only (`0x10604` shares
/// the pixel shader of `0x10204`) and is not computed here.
fn lod_passes(reflections: bool, screen_space_reflections: bool) -> SurfacePasses {
    use descriptor as bit;
    let ssr = reflections && screen_space_reflections;
    let main =
        bit::LOD | if reflections { bit::REFLECTIONS } else { 0 } | if ssr { bit::SSR } else { 0 };
    SurfacePasses {
        main_descriptor: main,
        variant: Variant::lod(ssr),
        depth_fog: None,
        mask_tag: None,
        unsupported: 0,
    }
}

/// Names of the unsupported bits for the log line.
pub fn describe_unsupported(bits: u32) -> String {
    let names = [
        (descriptor::VERTEX_ALPHA_DEPTH, "vertex-alpha depth"),
        (
            descriptor::WADING,
            "wading ripple on the refraction-only shader",
        ),
    ];
    names
        .iter()
        .filter(|(bit, _)| bits & bit != 0)
        .map(|(_, name)| *name)
        .collect::<Vec<_>>()
        .join(", ")
}

#[cfg(test)]
mod tests {
    use super::*;

    const EXTERIOR: PassContext = PassContext {
        interior: false,
        underwater: false,
        screen_space_reflections: false,
    };
    const EXTERIOR_SSR: PassContext = PassContext {
        screen_space_reflections: true,
        ..EXTERIOR
    };
    /// Flags 452 of the placed water meshes: depth, reflections, refractions,
    /// vertex uv.
    const MESH_FLAGS: u32 = 452;

    #[test]
    fn placed_mesh_gets_the_depth_descriptor_and_a_depth_fog_pass() {
        let passes = select(MESH_FLAGS, EXTERIOR);
        assert_eq!(passes.main_descriptor, 0x1d);
        assert_eq!(passes.depth_fog, Some(0x18011));
        assert_eq!(passes.variant, Variant::depth(false));
        assert_eq!(passes.mask_tag, Some(MaskTag::Surface));
        assert_eq!(passes.unsupported, 0);
    }

    #[test]
    fn interior_selects_the_no_sun_permutation_through_bit_0x20() {
        let passes = select(
            MESH_FLAGS,
            PassContext {
                interior: true,
                ..EXTERIOR
            },
        );
        assert_eq!(passes.main_descriptor, 0x3d);
        assert_eq!(passes.variant, Variant::depth(true));
    }

    #[test]
    fn the_refraction_only_shader_has_no_interior_permutation() {
        let passes = select(
            water_flags::REFLECTIONS | water_flags::REFRACTIONS,
            PassContext {
                interior: true,
                ..EXTERIOR
            },
        );
        assert_eq!(passes.main_descriptor, 0x2c);
        assert_eq!(passes.variant, Variant::refraction_only());
    }

    #[test]
    fn a_surface_without_the_depth_flag_uses_the_refraction_only_shader_and_no_depth_fog() {
        let passes = select(
            water_flags::REFLECTIONS | water_flags::REFRACTIONS,
            EXTERIOR,
        );
        assert_eq!(passes.main_descriptor, 0xc);
        assert_eq!(passes.variant, Variant::refraction_only());
        assert_eq!(passes.depth_fog, None);
    }

    #[test]
    fn camera_underwater_overrides_the_property_and_drops_depth() {
        let underwater = PassContext {
            underwater: true,
            ..EXTERIOR
        };
        let passes = select(MESH_FLAGS, underwater);
        assert_eq!(passes.main_descriptor, 0x180d);
        assert!(!passes.variant.depth);
        assert!(passes.variant.underwater);
        assert_eq!(passes.depth_fog, None);
        assert_eq!(passes.unsupported, 0);
    }

    #[test]
    fn underwater_with_ssr_keeps_the_underwater_variant_and_adds_the_ssr_bit() {
        let underwater = PassContext {
            underwater: true,
            ..EXTERIOR_SSR
        };
        let passes = select(MESH_FLAGS, underwater);
        assert_eq!(passes.main_descriptor, 0x1a0d);
        assert_eq!(passes.variant, Variant::underwater());
    }

    #[test]
    fn the_displacement_quad_under_water_uses_the_underwater_shader_like_its_source() {
        let underwater = PassContext {
            underwater: true,
            ..EXTERIOR
        };
        let passes = select(displacement_quad_flags(MESH_FLAGS), underwater);
        assert_eq!(passes.variant, Variant::underwater());
        assert!(passes.reads_displacement());
    }

    #[test]
    fn underwater_wading_reads_the_displacement_field_and_has_a_ripple_permutation() {
        let underwater = PassContext {
            underwater: true,
            ..EXTERIOR
        };
        let passes = select(MESH_FLAGS | water_flags::DISPLACEMENT, underwater);
        assert!(passes.reads_displacement());
        assert_eq!(passes.unsupported, 0);
        let refraction_only = select(
            water_flags::DISPLACEMENT | water_flags::REFRACTIONS,
            underwater,
        );
        assert!(refraction_only.reads_displacement());
        assert_eq!(refraction_only.unsupported, 0);
    }

    #[test]
    fn vertex_alpha_depth_is_ported_under_water_only() {
        let flags = MESH_FLAGS | water_flags::VERTEX_ALPHA_DEPTH;
        let underwater = PassContext {
            underwater: true,
            ..EXTERIOR
        };
        assert_eq!(select(flags, underwater).unsupported, 0);
        assert_eq!(
            select(flags, EXTERIOR).unsupported,
            descriptor::VERTEX_ALPHA_DEPTH
        );
    }

    #[test]
    fn the_mask_tag_passes_still_run_under_water() {
        let underwater = PassContext {
            underwater: true,
            ..EXTERIOR
        };
        assert_eq!(
            select(MESH_FLAGS, underwater).mask_tag,
            Some(MaskTag::Surface)
        );
    }

    #[test]
    fn the_propertys_own_underwater_bit_is_ignored_while_the_camera_is_above_water() {
        let passes = select(MESH_FLAGS | water_flags::UNDERWATER, EXTERIOR);
        assert_eq!(passes, select(MESH_FLAGS, EXTERIOR));
    }

    #[test]
    fn wading_surfaces_use_the_wading_mask_tag_and_no_depth_fog() {
        let passes = select(MESH_FLAGS | water_flags::DISPLACEMENT, EXTERIOR);
        assert_eq!(
            passes.main_descriptor & descriptor::WADING,
            descriptor::WADING
        );
        assert_eq!(passes.mask_tag, Some(MaskTag::Wading));
        assert_eq!(MaskTag::Wading.value(), 3);
        assert_eq!(passes.depth_fog, None);
        assert_eq!(
            passes.unsupported, 0,
            "the wading flag does not drop the depth bit"
        );
        assert!(passes.reads_displacement());
    }

    #[test]
    fn the_displacement_quad_reads_the_field_and_inherits_the_reflection_bits() {
        let flags = displacement_quad_flags(
            water_flags::REFLECTIONS | water_flags::VERTEX_UV | water_flags::LOD,
        );
        assert_eq!(
            flags,
            water_flags::DISPLACEMENT | water_flags::DEPTH | water_flags::REFLECTIONS
        );
        let passes = select(flags, EXTERIOR);
        assert!(passes.reads_displacement());
        assert_eq!(passes.mask_tag, Some(MaskTag::Wading));
        assert_eq!(passes.unsupported, 0);
    }

    #[test]
    fn only_wading_surfaces_read_the_displacement_field() {
        assert!(!select(MESH_FLAGS, EXTERIOR).reads_displacement());
    }

    #[test]
    fn wading_without_the_depth_shader_has_no_ripple_permutation_and_says_so() {
        let passes = select(
            water_flags::DISPLACEMENT | water_flags::REFRACTIONS,
            EXTERIOR,
        );
        assert!(!passes.reads_displacement());
        assert_eq!(passes.unsupported, descriptor::WADING);
    }

    #[test]
    fn vertex_alpha_depth_removes_the_depth_shader_and_is_reported() {
        let passes = select(MESH_FLAGS | water_flags::VERTEX_ALPHA_DEPTH, EXTERIOR);
        assert!(!passes.variant.depth);
        assert_eq!(passes.unsupported, descriptor::VERTEX_ALPHA_DEPTH);
    }

    #[test]
    fn lod_water_selects_the_lod_shader_with_no_extra_passes() {
        let passes = select(water_flags::LOD | water_flags::REFLECTIONS, EXTERIOR);
        assert_eq!(passes.main_descriptor, 0x10004);
        assert!(passes.variant.lod);
        assert_eq!(passes.depth_fog, None);
        assert_eq!(passes.mask_tag, None);
        assert_eq!(passes.unsupported, 0);
    }

    #[test]
    fn lod_water_ignores_interior_underwater_and_depth_state() {
        let flags = water_flags::LOD | water_flags::REFLECTIONS | water_flags::DEPTH;
        let underwater = PassContext {
            interior: true,
            underwater: true,
            ..EXTERIOR
        };
        assert_eq!(select(flags, underwater), select(flags, EXTERIOR));
        assert_eq!(select(flags, EXTERIOR).main_descriptor, 0x10004);
    }

    #[test]
    fn lod_water_without_the_reflections_flag_is_descriptor_0x10000() {
        assert_eq!(select(water_flags::LOD, EXTERIOR).main_descriptor, 0x10000);
    }

    #[test]
    fn lod_water_with_ssr_is_descriptor_0x10204_and_selects_the_ssr_taps() {
        let flags = water_flags::LOD | water_flags::REFLECTIONS;
        let passes = select(flags, EXTERIOR_SSR);
        assert_eq!(passes.main_descriptor, 0x10204);
        assert_eq!(passes.variant, Variant::lod(true));
        assert_eq!(passes.mask_tag, None);
        assert_eq!(passes.unsupported, 0);
    }

    #[test]
    fn lod_water_needs_the_reflections_flag_for_the_ssr_bit() {
        let passes = select(water_flags::LOD, EXTERIOR_SSR);
        assert_eq!(passes.main_descriptor, 0x10000);
        assert_eq!(passes.variant, Variant::lod(false));
    }

    #[test]
    fn lod_ssr_ignores_interior_and_underwater_state() {
        let flags = water_flags::LOD | water_flags::REFLECTIONS;
        let context = PassContext {
            interior: true,
            underwater: true,
            ..EXTERIOR_SSR
        };
        assert_eq!(select(flags, context), select(flags, EXTERIOR_SSR));
    }

    #[test]
    fn non_lod_water_carries_the_ssr_bit_without_changing_its_permutation() {
        let with = select(MESH_FLAGS, EXTERIOR_SSR);
        let without = select(MESH_FLAGS, EXTERIOR);
        assert_eq!(
            with.main_descriptor,
            without.main_descriptor | descriptor::SSR
        );
        assert_eq!(with.variant, without.variant);
    }

    #[test]
    fn variant_indices_are_a_bijection() {
        let mut seen = [false; Variant::COUNT];
        for variant in Variant::ALL {
            assert!(!seen[variant.index()]);
            seen[variant.index()] = true;
        }
    }

    #[test]
    fn the_unsupported_bits_are_named() {
        assert_eq!(
            describe_unsupported(descriptor::WADING | descriptor::VERTEX_ALPHA_DEPTH),
            "vertex-alpha depth, wading ripple on the refraction-only shader"
        );
    }
}
