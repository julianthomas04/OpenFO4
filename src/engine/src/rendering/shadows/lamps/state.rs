use super::settings::{LampShadowConfig, LampShadowSettings};

/// CPU authority for lamp shadows: the INI quality and the panel controls.
#[derive(Debug, Clone)]
pub struct LampShadowState {
    pub config: LampShadowConfig,
    pub settings: LampShadowSettings,
}

impl LampShadowState {
    pub fn new(config: LampShadowConfig) -> Self {
        Self {
            settings: LampShadowSettings::new(&config),
            config,
        }
    }
}
