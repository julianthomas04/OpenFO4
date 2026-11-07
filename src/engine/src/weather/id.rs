/// Position of a weather in the list of the weathers of one climate. Only the list that issued an
/// id can tell which weather it names.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct WeatherId(usize);

impl WeatherId {
    pub const fn new(index: usize) -> Self {
        Self(index)
    }

    pub const fn index(self) -> usize {
        self.0
    }
}

/// One weather a climate can pick and its relative chance.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ClimateEntry {
    pub weather: WeatherId,
    /// A weight against the other entries of the climate, not a percentage.
    pub chance: u32,
}
