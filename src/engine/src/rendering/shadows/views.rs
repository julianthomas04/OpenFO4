use glam::{Mat4, UVec4, Vec3, Vec4};

use crate::rendering::{frustum, lighting::frame_uniform::MAX_CASCADES};

/// Lamp views (cube faces or spot tiles) re-rendered per frame at most.
pub const MAX_LAMP_VIEWS_PER_FRAME: usize = 12;
/// Every view slot the shadow cull and the caster pipelines address: the sun
/// cascades first, then the lamp views of this frame.
pub const MAX_SHADOW_VIEWS: usize = MAX_CASCADES + MAX_LAMP_VIEWS_PER_FRAME;

/// Lamp views keep this caster size floor in shadow texels.
const LAMP_MIN_CASTER_TEXELS: f32 = 1.5;
/// `kind.y` bit: the view draws no alpha-tested casters. Mirrored by
/// `VIEW_FLAG_SKIP_ALPHA` in `shadow_cull.wgsl`.
const VIEW_FLAG_SKIP_ALPHA: u32 = 1;

/// Discriminants are mirrored by the `VIEW_*` constants in `shadow_cull.wgsl`.
#[derive(Debug, Copy, Clone, PartialEq, Eq)]
#[repr(u32)]
enum ViewKind {
    Unused = 0,
    Ortho = 1,
    Perspective = 2,
}

/// How the shadow cull rejects casters that cannot reach a view.
#[derive(Debug, Copy, Clone)]
pub enum ViewCull {
    /// A sun cascade: the light-aligned cylinder through the cascade sphere,
    /// with a fixed texel size.
    Ortho {
        sphere: Vec4,
        to_light: Vec3,
        texel_world: f32,
        /// Casters spanning fewer texels than this are not drawn.
        min_texels: f32,
        skip_alpha: bool,
    },
    /// A lamp view: casters must touch the light's range sphere; the texel
    /// size grows linearly with the distance from the light.
    Perspective {
        light_sphere: Vec4,
        texel_per_distance: f32,
        near: f32,
    },
}

/// One depth rendering of the scene from a light.
#[derive(Debug, Copy, Clone)]
pub struct ShadowView {
    pub view_proj: Mat4,
    pub cull: ViewCull,
}

/// Mirrors `ViewCull` in `shadow_cull.wgsl`.
#[repr(C)]
#[derive(Copy, Clone, bytemuck::Pod, bytemuck::Zeroable)]
pub struct GpuViewCull {
    planes: [Vec4; 6],
    /// Ortho: cascade sphere. Perspective: light position and range.
    bound: Vec4,
    /// Ortho: direction to the light.
    axis: Vec4,
    /// x: texel size (ortho) or texel size per unit distance (perspective),
    /// y: perspective near plane distance, z: minimum caster size in texels.
    metric: Vec4,
    /// x: `ViewKind`, y: `VIEW_FLAG_*` bits.
    kind: UVec4,
}

const _: () = assert!(std::mem::size_of::<GpuViewCull>() == 160);

impl GpuViewCull {
    pub const UNUSED: Self = Self {
        planes: [Vec4::ZERO; 6],
        bound: Vec4::ZERO,
        axis: Vec4::ZERO,
        metric: Vec4::ZERO,
        kind: UVec4::new(ViewKind::Unused as u32, 0, 0, 0),
    };
}

impl From<&ShadowView> for GpuViewCull {
    fn from(view: &ShadowView) -> Self {
        let planes = frustum::clip_planes(&view.view_proj);
        match view.cull {
            ViewCull::Ortho {
                sphere,
                to_light,
                texel_world,
                min_texels,
                skip_alpha,
            } => Self {
                planes,
                bound: sphere,
                axis: to_light.extend(0.0),
                metric: Vec4::new(texel_world, 0.0, min_texels, 0.0),
                kind: UVec4::new(
                    ViewKind::Ortho as u32,
                    if skip_alpha { VIEW_FLAG_SKIP_ALPHA } else { 0 },
                    0,
                    0,
                ),
            },
            ViewCull::Perspective {
                light_sphere,
                texel_per_distance,
                near,
            } => Self {
                planes,
                bound: light_sphere,
                axis: Vec4::ZERO,
                metric: Vec4::new(texel_per_distance, near, LAMP_MIN_CASTER_TEXELS, 0.0),
                kind: UVec4::new(ViewKind::Perspective as u32, 0, 0, 0),
            },
        }
    }
}

/// The views of one frame, each bound to its slot: cascade `i` in slot `i`,
/// lamp view `j` in slot `cascade_slots + j`.
#[derive(Debug, Clone, Default)]
pub struct ShadowViewSet {
    /// `None`: the cascade keeps last render, nothing is culled or drawn.
    pub cascades: Vec<Option<ShadowView>>,
    pub lamps: Vec<ShadowView>,
}

impl ShadowViewSet {
    pub fn is_empty(&self) -> bool {
        self.rendered_cascades() == 0 && self.lamps.is_empty()
    }

    /// Bit `i` is set when cascade `i` renders this frame.
    pub fn rendered_cascades(&self) -> u32 {
        self.cascades
            .iter()
            .enumerate()
            .filter(|(_, view)| view.is_some())
            .fold(0, |mask, (i, _)| mask | 1 << i)
    }

    /// `(slot, view)` for every view; lamp slots start after all configured
    /// cascade slots so the draw regions never move.
    pub fn slotted(&self, cascade_slots: usize) -> impl Iterator<Item = (usize, &ShadowView)> {
        self.cascades
            .iter()
            .enumerate()
            .filter_map(|(slot, view)| view.as_ref().map(|view| (slot, view)))
            .chain(
                self.lamps
                    .iter()
                    .enumerate()
                    .map(move |(j, view)| (cascade_slots + j, view)),
            )
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn view() -> ShadowView {
        ShadowView {
            view_proj: Mat4::orthographic_rh(-10.0, 10.0, -10.0, 10.0, 10.0, -100.0),
            cull: ViewCull::Ortho {
                sphere: Vec4::new(1.0, 2.0, 3.0, 4.0),
                to_light: Vec3::Y,
                texel_world: 0.25,
                min_texels: 3.0,
                skip_alpha: false,
            },
        }
    }

    fn lamp_view() -> ShadowView {
        ShadowView {
            view_proj: Mat4::IDENTITY,
            cull: ViewCull::Perspective {
                light_sphere: Vec4::new(5.0, 6.0, 7.0, 8.0),
                texel_per_distance: 0.01,
                near: 0.5,
            },
        }
    }

    fn view_set(cascades: &[bool], lamps: usize) -> ShadowViewSet {
        ShadowViewSet {
            cascades: cascades
                .iter()
                .map(|rendered| rendered.then(view))
                .collect(),
            lamps: vec![lamp_view(); lamps],
        }
    }

    #[test]
    fn rendered_cascades_sets_one_bit_per_rendered_cascade() {
        assert_eq!(view_set(&[true, false, true], 0).rendered_cascades(), 0b101);
        assert_eq!(view_set(&[false, false, false], 0).rendered_cascades(), 0);
    }

    #[test]
    fn a_set_is_empty_only_without_any_cascade_or_lamp_view() {
        assert!(view_set(&[false, false], 0).is_empty());
        assert!(ShadowViewSet::default().is_empty());
        assert!(!view_set(&[false], 1).is_empty());
        assert!(!view_set(&[false, true], 0).is_empty());
    }

    #[test]
    fn slotted_keeps_cascade_slots_and_puts_lamps_after_all_configured_cascades() {
        let set = view_set(&[true, false, true], 2);
        let slots: Vec<usize> = set.slotted(4).map(|(slot, _)| slot).collect();
        assert_eq!(slots, vec![0, 2, 4, 5]);
    }

    #[test]
    fn an_ortho_view_uploads_its_sphere_axis_and_texel_metrics() {
        let gpu = GpuViewCull::from(&view());
        assert_eq!(gpu.bound, Vec4::new(1.0, 2.0, 3.0, 4.0));
        assert_eq!(gpu.axis, Vec4::new(0.0, 1.0, 0.0, 0.0));
        assert_eq!(gpu.metric, Vec4::new(0.25, 0.0, 3.0, 0.0));
        assert_eq!(gpu.kind, UVec4::new(1, 0, 0, 0));
    }

    #[test]
    fn an_ortho_view_that_skips_alpha_sets_the_flag_bit() {
        let mut view = view();
        if let ViewCull::Ortho { skip_alpha, .. } = &mut view.cull {
            *skip_alpha = true;
        }
        assert_eq!(GpuViewCull::from(&view).kind.y, VIEW_FLAG_SKIP_ALPHA);
    }

    #[test]
    fn a_perspective_view_uploads_the_light_sphere_and_the_lamp_caster_floor() {
        let gpu = GpuViewCull::from(&lamp_view());
        assert_eq!(gpu.bound, Vec4::new(5.0, 6.0, 7.0, 8.0));
        assert_eq!(
            gpu.metric,
            Vec4::new(0.01, 0.5, LAMP_MIN_CASTER_TEXELS, 0.0)
        );
        assert_eq!(gpu.kind, UVec4::new(2, 0, 0, 0));
    }
}
