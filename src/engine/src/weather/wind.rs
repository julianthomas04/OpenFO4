#[derive(Debug, Clone, Copy, PartialEq)]
pub struct WindParams {
    /// 0..1.
    pub speed: f32,
    /// 0..1; `None` when the weather's data predates the field.
    pub turbulence: Option<f32>,
    /// The record's unscaled byte.
    pub direction_raw: u8,
    /// The record's unscaled byte.
    pub direction_range_raw: u8,
}
