//! The `water:WF:F1:F2:UOU,UOV:USU,USV` material-name token: the fields of a
//! NIF `BSWaterShaderProperty` the installer carries into the GLB.

use glam::Vec2;

/// Bits of the NIF `WaterShaderPropertyFlags` (nif.xml).
pub mod water_flags {
    pub const DISPLACEMENT: u32 = 1 << 0;
    pub const LOD: u32 = 1 << 1;
    pub const DEPTH: u32 = 1 << 2;
    pub const ACTOR_IN_WATER: u32 = 1 << 3;
    pub const ACTOR_IN_WATER_IS_MOVING: u32 = 1 << 4;
    pub const UNDERWATER: u32 = 1 << 5;
    pub const REFLECTIONS: u32 = 1 << 6;
    pub const REFRACTIONS: u32 = 1 << 7;
    pub const VERTEX_UV: u32 = 1 << 8;
    pub const VERTEX_ALPHA_DEPTH: u32 = 1 << 9;
    pub const PROCEDURAL: u32 = 1 << 10;
    pub const FOG: u32 = 1 << 11;
    pub const UPDATE_CONSTANTS: u32 = 1 << 12;
    pub const CUBEMAP: u32 = 1 << 13;
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct WaterShaderToken {
    /// Raw `WaterShaderPropertyFlags`, see [`water_flags`].
    pub water_flags: u32,
    /// Raw shader flag words 1 and 2 of the property.
    pub shader_flags_1: u32,
    pub shader_flags_2: u32,
    pub uv_offset: Vec2,
    pub uv_scale: Vec2,
}

impl WaterShaderToken {
    pub fn has_flag(&self, flag: u32) -> bool {
        self.water_flags & flag != 0
    }

    /// Decodes the token value, the text after `water:`.
    pub fn parse(value: &str) -> Option<Self> {
        let mut fields = value.split(':');
        let water_flags = fields.next()?.parse::<u32>().ok()?;
        let shader_flags_1 = parse_hex_word(fields.next()?)?;
        let shader_flags_2 = parse_hex_word(fields.next()?)?;
        let uv_offset = parse_pair(fields.next()?)?;
        let uv_scale = parse_pair(fields.next()?)?;
        if fields.next().is_some() {
            return None;
        }
        Some(Self {
            water_flags,
            shader_flags_1,
            shader_flags_2,
            uv_offset,
            uv_scale,
        })
    }
}

fn parse_hex_word(text: &str) -> Option<u32> {
    if text.len() != 8 || !text.bytes().all(|b| b.is_ascii_hexdigit()) {
        return None;
    }
    u32::from_str_radix(text, 16).ok()
}

fn parse_pair(text: &str) -> Option<Vec2> {
    let (u, v) = text.split_once(',')?;
    let component = |s: &str| s.parse::<f32>().ok().filter(|x| x.is_finite());
    Some(Vec2::new(component(u)?, component(v)?))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_every_field_of_the_installer_token() {
        let token = WaterShaderToken::parse("452:80000008:00000001:0.5,-0.25:1,2.5").unwrap();
        assert_eq!(token.water_flags, 452);
        assert_eq!(token.shader_flags_1, 0x8000_0008);
        assert_eq!(token.shader_flags_2, 1);
        assert_eq!(token.uv_offset, Vec2::new(0.5, -0.25));
        assert_eq!(token.uv_scale, Vec2::new(1.0, 2.5));
    }

    #[test]
    fn flags_452_are_depth_reflections_refractions_and_vertex_uv() {
        let token = WaterShaderToken::parse("452:00000000:00000000:0,0:1,1").unwrap();
        for flag in [
            water_flags::DEPTH,
            water_flags::REFLECTIONS,
            water_flags::REFRACTIONS,
            water_flags::VERTEX_UV,
        ] {
            assert!(token.has_flag(flag), "{flag:#x}");
        }
        assert!(!token.has_flag(water_flags::VERTEX_ALPHA_DEPTH));
    }

    #[test]
    fn rejects_malformed_values() {
        for bad in [
            "",
            "452",
            "x:00000000:00000000:0,0:1,1",
            "452:0:00000000:0,0:1,1",
            "452:0000000G:00000000:0,0:1,1",
            "452:00000000:00000000:0:1,1",
            "452:00000000:00000000:0,0:1,nan",
            "452:00000000:00000000:0,0:1,1:extra",
            "-1:00000000:00000000:0,0:1,1",
        ] {
            assert_eq!(WaterShaderToken::parse(bad), None, "{bad:?}");
        }
    }
}
