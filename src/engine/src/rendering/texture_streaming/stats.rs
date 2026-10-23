/// Texture streaming counters for the PERF line. Gauges describe the current
/// state; everything else is monotonic and reported as a per-interval delta.
#[derive(Debug, Default, Clone, Copy, PartialEq)]
pub struct TextureStats {
    pub resident_bytes: u64,
    pub live_slots: u32,
    pub pending: u32,
    pub upgrades: u64,
    pub downgrades: u64,
    pub uploaded_bytes: u64,
    pub placeholders: u64,
    pub slots_recycled: u64,
    pub bind_group_rebuilds: u64,
    pub stale_dropped: u64,
    /// Upgrades the GPU feedback asked for beyond what the estimate wanted.
    pub feedback_upgrades: u64,
    /// Resident textures that wanted a finer mip for longer than the stuck
    /// threshold; only measured with `OPENFO4_TEX_STUCK_LOG=1`.
    pub stuck: u32,
    /// Resident textures coarser than full resolution that nothing has asked
    /// for, longer than the stuck threshold; measured with the stuck log.
    pub undemanded: u32,
    /// Milliseconds since the last feedback readback was processed; `None`
    /// while feedback is off or has not reported yet.
    pub feedback_age_ms: Option<u32>,
}
