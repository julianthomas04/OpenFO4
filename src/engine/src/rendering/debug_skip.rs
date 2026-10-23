//! Debug-only knobs that attribute CPU encode cost to individual passes.

use crate::utils::{env_opt, env_or};

const SKIP_PASSES_ENV: &str = "OPENFO4_DEBUG_SKIP_PASSES";
const MATERIAL_ARRAY_LEN_ENV: &str = "OPENFO4_DEBUG_MATERIAL_ARRAY_LEN";

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum DebugPass {
    Cull,
    Lights,
    Shadows,
    World,
    Water,
    WaterDisplacement,
    WaterSsr,
    Hiz,
    DepthOfField,
    Exposure,
    Tonemap,
    Hud,
    PipBoy,
    Egui,
}

impl DebugPass {
    const ALL: [(&'static str, DebugPass); 14] = [
        ("cull", DebugPass::Cull),
        ("lights", DebugPass::Lights),
        ("shadows", DebugPass::Shadows),
        ("world", DebugPass::World),
        ("water", DebugPass::Water),
        ("water_displacement", DebugPass::WaterDisplacement),
        ("water_ssr", DebugPass::WaterSsr),
        ("hiz", DebugPass::Hiz),
        ("depth_of_field", DebugPass::DepthOfField),
        ("exposure", DebugPass::Exposure),
        ("tonemap", DebugPass::Tonemap),
        ("hud", DebugPass::Hud),
        ("pipboy", DebugPass::PipBoy),
        ("egui", DebugPass::Egui),
    ];

    fn bit(self) -> u16 {
        1 << self as u16
    }
}

/// Passes whose encoding is skipped, parsed once from `OPENFO4_DEBUG_SKIP_PASSES`.
#[derive(Clone, Copy, Default)]
pub struct DebugSkip {
    mask: u16,
}

impl DebugSkip {
    pub fn from_env() -> Self {
        let raw: String = env_or(SKIP_PASSES_ENV, String::new());
        let mut mask = 0;
        for name in raw
            .split(',')
            .map(|n| n.trim().to_ascii_lowercase())
            .filter(|n| !n.is_empty())
        {
            if let Some((_, pass)) = DebugPass::ALL.iter().find(|(known, _)| *known == name) {
                mask |= pass.bit();
            } else {
                let known: Vec<_> = DebugPass::ALL.iter().map(|(n, _)| *n).collect();
                tracing::warn!(
                    "{SKIP_PASSES_ENV}: unknown pass {name:?} (known: {})",
                    known.join(",")
                );
            }
        }
        if mask != 0 {
            tracing::warn!("{SKIP_PASSES_ENV}={raw:?}: passes are skipped, the image is wrong");
        }
        Self { mask }
    }

    pub fn skips(&self, pass: DebugPass) -> bool {
        self.mask & pass.bit() != 0
    }
}

/// `OPENFO4_DEBUG_MATERIAL_ARRAY_LEN`: only the first N material array views
/// are bound, the rest are stand-ins. `None` when unset.
pub fn material_array_len() -> Option<u32> {
    static LEN: std::sync::OnceLock<Option<u32>> = std::sync::OnceLock::new();
    *LEN.get_or_init(|| {
        let len = env_opt::<u32>(MATERIAL_ARRAY_LEN_ENV)?;
        tracing::warn!(
            "{MATERIAL_ARRAY_LEN_ENV}={len}: material views past it are stand-ins, textures are wrong"
        );
        Some(len)
    })
}
