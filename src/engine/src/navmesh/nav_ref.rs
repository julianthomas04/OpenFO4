/// Generation-checked triangle reference: `slot` (32 bits) | `salt` (16) | `tri` (16).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct NavRef(u64);

impl NavRef {
    pub const fn new(slot: u32, salt: u16, tri: u16) -> Self {
        Self((slot as u64) << 32 | (salt as u64) << 16 | tri as u64)
    }

    pub const fn slot(self) -> u32 {
        (self.0 >> 32) as u32
    }

    pub const fn salt(self) -> u16 {
        (self.0 >> 16) as u16
    }

    pub const fn tri(self) -> u16 {
        self.0 as u16
    }
}
