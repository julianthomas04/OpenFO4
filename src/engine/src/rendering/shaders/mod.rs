pub mod composed;
pub mod composition;
#[cfg(test)]
mod lighting_tests;
pub mod startup;
pub mod translated;

pub struct ShaderSource {
    label: &'static str,
    wgsl: &'static str,
}

impl ShaderSource {
    pub fn wgsl(&self) -> &'static str {
        self.wgsl
    }

    pub fn compile(&self, device: &wgpu::Device) -> wgpu::ShaderModule {
        device.create_shader_module(wgpu::ShaderModuleDescriptor {
            label: Some(self.label),
            source: wgpu::ShaderSource::Wgsl(self.wgsl.into()),
        })
    }
}

pub const OBJECT: ShaderSource = ShaderSource {
    label: "object_shader",
    wgsl: concat!(
        include_str!("binding_array.wgsl"),
        include_str!("light_data.wgsl"),
        include_str!("frame.wgsl"),
        include_str!("instance.wgsl"),
        include_str!("scene_bindings.wgsl"),
        include_str!("env_bindings.wgsl"),
        include_str!("vertex_io.wgsl"),
        include_str!("normal_mapping.wgsl"),
        include_str!("shading_math.wgsl"),
        include_str!("ambient.wgsl"),
        include_str!("surface_shading.wgsl"),
        include_str!("lamp_shadow.wgsl"),
        include_str!("mip_debug.wgsl"),
        include_str!("texture_feedback.wgsl"),
        include_str!("lighting.wgsl"),
        include_str!("surface.wgsl"),
        include_str!("env_reflection.wgsl"),
        include_str!("effect_shading.wgsl"),
        include_str!("object.wgsl"),
    ),
};

pub const TERRAIN: ShaderSource = ShaderSource {
    label: "terrain_shader",
    wgsl: concat!(
        include_str!("binding_array.wgsl"),
        include_str!("light_data.wgsl"),
        include_str!("frame.wgsl"),
        include_str!("instance.wgsl"),
        include_str!("scene_bindings.wgsl"),
        include_str!("vertex_io.wgsl"),
        include_str!("shading_math.wgsl"),
        include_str!("ambient.wgsl"),
        include_str!("surface_shading.wgsl"),
        include_str!("lamp_shadow.wgsl"),
        include_str!("mip_debug.wgsl"),
        include_str!("texture_feedback.wgsl"),
        include_str!("lighting.wgsl"),
        include_str!("terrain.wgsl"),
    ),
};

pub const SKY: ShaderSource = ShaderSource {
    label: "sky_shader",
    wgsl: concat!(
        include_str!("light_data.wgsl"),
        include_str!("frame.wgsl"),
        include_str!("fullscreen.wgsl"),
        include_str!("sky.wgsl"),
    ),
};

pub const SKY_MOON: ShaderSource = ShaderSource {
    label: "sky_moon_shader",
    wgsl: concat!(
        include_str!("light_data.wgsl"),
        include_str!("frame.wgsl"),
        include_str!("sky_moon.wgsl"),
    ),
};

pub const SKY_SUN: ShaderSource = ShaderSource {
    label: "sky_sun_shader",
    wgsl: concat!(
        include_str!("light_data.wgsl"),
        include_str!("frame.wgsl"),
        include_str!("sky_sun.wgsl"),
    ),
};

pub const SKY_STARS: ShaderSource = ShaderSource {
    label: "sky_stars_shader",
    wgsl: concat!(
        include_str!("light_data.wgsl"),
        include_str!("frame.wgsl"),
        include_str!("sky_stars.wgsl"),
    ),
};

pub const SKY_CLOUDS: ShaderSource = ShaderSource {
    label: "sky_clouds_shader",
    wgsl: concat!(
        include_str!("light_data.wgsl"),
        include_str!("frame.wgsl"),
        include_str!("sky_clouds.wgsl"),
    ),
};

pub const TONEMAP: ShaderSource = ShaderSource {
    label: "tonemap_shader",
    wgsl: concat!(
        include_str!("light_data.wgsl"),
        include_str!("frame.wgsl"),
        include_str!("fullscreen.wgsl"),
        include_str!("tonemap.wgsl"),
    ),
};

pub const DEBUG_LINES: ShaderSource = ShaderSource {
    label: "debug_lines_shader",
    wgsl: concat!(
        include_str!("light_data.wgsl"),
        include_str!("frame.wgsl"),
        include_str!("debug_lines.wgsl"),
    ),
};

pub const SKELETON_OVERLAY: ShaderSource = ShaderSource {
    label: "skeleton_overlay_shader",
    wgsl: concat!(
        include_str!("light_data.wgsl"),
        include_str!("frame.wgsl"),
        include_str!("skeleton_overlay.wgsl"),
    ),
};

pub const OBJECT_CULL: ShaderSource = ShaderSource {
    label: "object_cull_shader",
    wgsl: concat!(
        include_str!("instance.wgsl"),
        include_str!("indirect.wgsl"),
        include_str!("hiz_occlusion.wgsl"),
        include_str!("gpu_cull.wgsl"),
    ),
};

pub const TERRAIN_CULL: ShaderSource = ShaderSource {
    label: "terrain_cull_shader",
    wgsl: concat!(
        include_str!("instance.wgsl"),
        include_str!("indirect.wgsl"),
        include_str!("terrain_cull.wgsl"),
    ),
};

pub const SHADOW: ShaderSource = ShaderSource {
    label: "shadow_shader",
    wgsl: concat!(
        include_str!("binding_array.wgsl"),
        include_str!("instance.wgsl"),
        include_str!("scene_bindings.wgsl"),
        include_str!("vertex_io.wgsl"),
        include_str!("shadow.wgsl"),
    ),
};

pub const SHADOW_TILE_CLEAR: ShaderSource = ShaderSource {
    label: "shadow_tile_clear_shader",
    wgsl: concat!(
        include_str!("fullscreen.wgsl"),
        include_str!("shadow_tile_clear.wgsl"),
    ),
};

pub const LAMP_ATLAS_PREVIEW: ShaderSource = ShaderSource {
    label: "lamp_atlas_preview_shader",
    wgsl: concat!(
        include_str!("fullscreen.wgsl"),
        include_str!("lamp_atlas_preview.wgsl"),
    ),
};

pub const SHADOW_CULL: ShaderSource = ShaderSource {
    label: "shadow_cull_shader",
    wgsl: concat!(
        include_str!("instance.wgsl"),
        include_str!("indirect.wgsl"),
        include_str!("shadow_cull.wgsl"),
    ),
};

pub const HIZ_DEPTH_COPY: ShaderSource = ShaderSource {
    label: "hiz_depth_copy_shader",
    wgsl: include_str!("hiz_depth_copy.wgsl"),
};

pub const HIZ_REDUCE: ShaderSource = ShaderSource {
    label: "hiz_reduce_shader",
    wgsl: include_str!("hiz_reduce.wgsl"),
};

pub const AUTO_EXPOSURE: ShaderSource = ShaderSource {
    label: "auto_exposure_shader",
    wgsl: include_str!("auto_exposure.wgsl"),
};

pub const HUD: ShaderSource = ShaderSource {
    label: "hud_shader",
    wgsl: include_str!("hud.wgsl"),
};

pub const MENU_VIDEO: ShaderSource = ShaderSource {
    label: "menu_video_shader",
    wgsl: include_str!("menu_video.wgsl"),
};

pub const MENU_ICON: ShaderSource = ShaderSource {
    label: "menu_icon_shader",
    wgsl: include_str!("menu_icon.wgsl"),
};

pub const LIGHT_SPHERES: ShaderSource = ShaderSource {
    label: "light_spheres_shader",
    wgsl: concat!(
        include_str!("light_data.wgsl"),
        include_str!("frame.wgsl"),
        include_str!("light_spheres.wgsl"),
    ),
};

pub const LIGHT_CULL: ShaderSource = ShaderSource {
    label: "light_cull_shader",
    wgsl: concat!(
        include_str!("light_data.wgsl"),
        include_str!("hiz_occlusion.wgsl"),
        include_str!("light_cull.wgsl"),
    ),
};

pub const SKINNED_OBJECT: ShaderSource = ShaderSource {
    label: "skinned_object_shader",
    wgsl: concat!(
        include_str!("binding_array.wgsl"),
        include_str!("light_data.wgsl"),
        include_str!("frame.wgsl"),
        include_str!("instance.wgsl"),
        include_str!("scene_bindings.wgsl"),
        include_str!("env_bindings.wgsl"),
        include_str!("vertex_io.wgsl"),
        include_str!("normal_mapping.wgsl"),
        include_str!("shading_math.wgsl"),
        include_str!("ambient.wgsl"),
        include_str!("surface_shading.wgsl"),
        include_str!("lamp_shadow.wgsl"),
        include_str!("mip_debug.wgsl"),
        include_str!("texture_feedback.wgsl"),
        include_str!("lighting.wgsl"),
        include_str!("surface.wgsl"),
        include_str!("env_reflection.wgsl"),
        include_str!("effect_shading.wgsl"),
        include_str!("object.wgsl"),
        include_str!("skinning.wgsl"),
        include_str!("skinned_object.wgsl"),
    ),
};

pub const SKINNED_SHADOW: ShaderSource = ShaderSource {
    label: "skinned_shadow_shader",
    wgsl: concat!(
        include_str!("binding_array.wgsl"),
        include_str!("instance.wgsl"),
        include_str!("scene_bindings.wgsl"),
        include_str!("vertex_io.wgsl"),
        include_str!("shadow.wgsl"),
        include_str!("skinning.wgsl"),
        include_str!("skinned_shadow.wgsl"),
    ),
};

pub const LIGHT_CLUSTER: ShaderSource = ShaderSource {
    label: "light_cluster_shader",
    wgsl: concat!(
        include_str!("light_data.wgsl"),
        include_str!("light_cluster.wgsl"),
    ),
};

#[cfg(test)]
const ALL: &[&ShaderSource] = &[
    &OBJECT,
    &TERRAIN,
    &SKY,
    &SKY_MOON,
    &SKY_SUN,
    &SKY_STARS,
    &SKY_CLOUDS,
    &TONEMAP,
    &DEBUG_LINES,
    &SKELETON_OVERLAY,
    &OBJECT_CULL,
    &TERRAIN_CULL,
    &SHADOW,
    &SHADOW_TILE_CLEAR,
    &LAMP_ATLAS_PREVIEW,
    &SHADOW_CULL,
    &HIZ_DEPTH_COPY,
    &HIZ_REDUCE,
    &AUTO_EXPOSURE,
    &HUD,
    &MENU_VIDEO,
    &MENU_ICON,
    &LIGHT_SPHERES,
    &LIGHT_CULL,
    &SKINNED_OBJECT,
    &SKINNED_SHADOW,
    &LIGHT_CLUSTER,
];

#[cfg(test)]
mod tests {
    use super::*;
    use naga::valid::{Capabilities, ValidationFlags, Validator};

    #[test]
    fn every_composed_shader_parses_and_validates() {
        for shader in ALL {
            let source = shader.wgsl();
            let module = naga::front::wgsl::parse_str(source)
                .unwrap_or_else(|e| panic!("{} parse: {}", shader.label, e.emit_to_string(source)));
            Validator::new(ValidationFlags::all(), Capabilities::all())
                .validate(&module)
                .unwrap_or_else(|e| {
                    panic!("{} validate: {}", shader.label, e.emit_to_string(source))
                });
        }
    }

    #[test]
    fn shader_registry_lists_every_declared_source() {
        let declared = include_str!("mod.rs")
            .lines()
            .filter(|line| line.starts_with("pub const "))
            .count();
        assert_eq!(ALL.len(), declared);
    }
}
