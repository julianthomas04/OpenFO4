use std::sync::mpsc;

use crate::rendering::device_loss::{DeviceLossMonitor, DeviceLost};

#[derive(Debug, thiserror::Error)]
pub enum ReadbackError {
    #[error(transparent)]
    DeviceLost(#[from] DeviceLost),
    #[error("readback poll failed: {0}")]
    Poll(#[from] wgpu::PollError),
    #[error("{label} staging slot {slot} failed to map: {source}")]
    Map {
        label: &'static str,
        slot: usize,
        #[source]
        source: wgpu::BufferAsyncError,
    },
    #[error("{label} staging slot {slot} mapped range unavailable: {source}")]
    MappedRange {
        label: &'static str,
        slot: usize,
        #[source]
        source: wgpu::MapRangeError,
    },
}

/// Device loss must stop the frame loop; every other readback failure only
/// costs one stale sample and is logged.
pub fn escalate_device_loss(result: Result<(), ReadbackError>) -> Result<(), DeviceLost> {
    match result {
        Ok(()) => Ok(()),
        Err(ReadbackError::DeviceLost(lost)) => Err(lost),
        Err(recoverable) => {
            tracing::error!("{recoverable}");
            Ok(())
        }
    }
}

type MapOutcome = (usize, Result<(), wgpu::BufferAsyncError>);

pub const SLOTS: usize = 3;

/// Staging ring copying GPU results back to the CPU without ever blocking: a
/// slot still mapped or in flight just skips a frame.
pub struct ReadbackRing {
    label: &'static str,
    staging: [wgpu::Buffer; SLOTS],
    map_sender: mpsc::Sender<MapOutcome>,
    map_outcomes: mpsc::Receiver<MapOutcome>,
    in_flight: [bool; SLOTS],
    /// Slot handed out this frame; mapped in `post_submit`.
    acquired: Option<usize>,
    next: usize,
}

impl ReadbackRing {
    pub fn new(device: &wgpu::Device, label: &'static str, slot_bytes: u64) -> Self {
        let staging = std::array::from_fn(|i| {
            device.create_buffer(&wgpu::BufferDescriptor {
                label: Some(&format!("{label}_staging_{i}")),
                size: slot_bytes.next_multiple_of(4),
                usage: wgpu::BufferUsages::COPY_DST | wgpu::BufferUsages::MAP_READ,
                mapped_at_creation: false,
            })
        });

        let (map_sender, map_outcomes) = mpsc::channel();

        Self {
            label,
            staging,
            map_sender,
            map_outcomes,
            in_flight: [false; SLOTS],
            acquired: None,
            next: 0,
        }
    }

    /// The slot to copy this frame's results into, or `None` when the ring is
    /// full and the frame must skip the readback rather than block.
    pub fn acquire(&mut self) -> Option<usize> {
        if self.in_flight[self.next] {
            return None;
        }
        self.acquired = Some(self.next);
        self.acquired
    }

    pub fn buffer(&self, slot: usize) -> &wgpu::Buffer {
        &self.staging[slot]
    }

    /// Call after queue.submit: kicks off the async map for the slot acquired
    /// this frame.
    pub fn post_submit(&mut self) {
        let Some(slot) = self.acquired.take() else {
            return;
        };
        let sender = self.map_sender.clone();
        let label = self.label;
        self.staging[slot]
            .slice(..)
            .map_async(wgpu::MapMode::Read, move |result| {
                if let Err(mpsc::SendError((slot, _))) = sender.send((slot, result)) {
                    tracing::debug!("{label} readback dropped before slot {slot} mapped");
                }
            });
        self.in_flight[slot] = true;
        self.next = (slot + 1) % SLOTS;
    }

    /// Non-blocking: hands every completed slot's bytes to `on_slot`.
    pub fn poll(
        &mut self,
        device: &wgpu::Device,
        device_loss: &DeviceLossMonitor,
        mut on_slot: impl FnMut(usize, &[u8]),
    ) -> Result<(), ReadbackError> {
        if !self.in_flight.iter().any(|f| *f) {
            return Ok(());
        }

        device.poll(wgpu::PollType::Poll)?;
        // Device loss destroys the staging buffers even when their map succeeded.
        device_loss.check()?;

        loop {
            match self.map_outcomes.try_recv() {
                Ok((slot, result)) => self.harvest(slot, result, &mut on_slot)?,
                Err(mpsc::TryRecvError::Empty) => return Ok(()),
                Err(mpsc::TryRecvError::Disconnected) => {
                    unreachable!("the readback ring owns a map sender")
                }
            }
        }
    }

    fn harvest(
        &mut self,
        slot: usize,
        result: Result<(), wgpu::BufferAsyncError>,
        on_slot: &mut impl FnMut(usize, &[u8]),
    ) -> Result<(), ReadbackError> {
        self.in_flight[slot] = false;
        result.map_err(|source| ReadbackError::Map {
            label: self.label,
            slot,
            source,
        })?;

        let outcome = self.staging[slot]
            .slice(..)
            .get_mapped_range()
            .map(|data| on_slot(slot, &data))
            .map_err(|source| ReadbackError::MappedRange {
                label: self.label,
                slot,
                source,
            });
        self.staging[slot].unmap();
        outcome
    }
}
