//! How the viewer sees a loading screen's model: in its own colours or in the HUD colour alone.

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub enum ColourMode {
    /// The model's textures as they are.
    #[default]
    Own,
    /// Only the textures' brightness, in the HUD colour.
    Monochrome,
}

impl ColourMode {
    /// The other mode.
    #[must_use]
    pub fn toggled(self) -> Self {
        match self {
            Self::Own => Self::Monochrome,
            Self::Monochrome => Self::Own,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_model_starts_in_its_own_colours() {
        assert_eq!(ColourMode::default(), ColourMode::Own);
    }

    #[test]
    fn toggling_switches_between_the_two_modes() {
        assert_eq!(ColourMode::Own.toggled(), ColourMode::Monochrome);
        assert_eq!(ColourMode::Monochrome.toggled(), ColourMode::Own);
        assert_eq!(ColourMode::Own.toggled().toggled(), ColourMode::Own);
    }
}
