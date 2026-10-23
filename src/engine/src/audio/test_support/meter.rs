use std::sync::Arc;
use std::sync::atomic::{AtomicU32, Ordering};

use kira::Frame;
use kira::effect::{Effect, EffectBuilder};
use kira::info::Info;

struct PeakMeter(Arc<AtomicU32>);

impl Effect for PeakMeter {
    fn process(&mut self, input: &mut [Frame], _dt: f64, _info: &Info) {
        for frame in input {
            let peak = frame.left.abs().max(frame.right.abs());
            self.0.fetch_max(peak.to_bits(), Ordering::Relaxed);
        }
    }
}

/// An effect that records the loudest sample passing through its track.
pub struct PeakMeterBuilder(Arc<AtomicU32>);

impl EffectBuilder for PeakMeterBuilder {
    type Handle = ();

    fn build(self) -> (Box<dyn Effect>, Self::Handle) {
        (Box::new(PeakMeter(self.0)), ())
    }
}

/// Reads what a [`PeakMeterBuilder`]'s effect measured.
pub struct PeakReader(Arc<AtomicU32>);

impl PeakReader {
    /// The loudest sample since the last call.
    pub fn take(&self) -> f32 {
        f32::from_bits(self.0.swap(0, Ordering::Relaxed))
    }
}

pub fn peak_meter() -> (PeakMeterBuilder, PeakReader) {
    let peak = Arc::new(AtomicU32::new(0));
    (PeakMeterBuilder(peak.clone()), PeakReader(peak))
}
