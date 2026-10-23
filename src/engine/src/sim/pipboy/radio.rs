//! Radio stations shown on RADIO.

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct StationId(pub u16);

#[derive(Clone, Debug, PartialEq)]
pub struct RadioStation {
    pub id: StationId,
    pub name: String,
    pub frequency_mhz: f32,
    pub in_range: bool,
}

#[derive(Clone, Debug, Default, PartialEq)]
pub struct Radio {
    pub stations: Vec<RadioStation>,
    pub tuned: Option<StationId>,
}
