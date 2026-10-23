use std::collections::VecDeque;
use std::time::Instant;

pub struct FpsUtils {
    samples: VecDeque<f64>,
    max_samples: usize,
    last_frame: Instant,
}

impl FpsUtils {
    pub fn new(max_samples: usize) -> Self {
        Self {
            samples: VecDeque::with_capacity(max_samples),
            max_samples,
            last_frame: Instant::now(),
        }
    }

    /// Advance the frame clock: sample the time since the previous call and
    /// record it. Must run exactly once per frame — it is the sole source of
    /// `dt()`, which drives the sim and camera.
    pub fn tick(&mut self) {
        let now = Instant::now();
        let dt = now.duration_since(self.last_frame).as_secs_f64();
        self.last_frame = now;

        if self.samples.len() >= self.max_samples {
            self.samples.pop_front();
        }
        self.samples.push_back(dt);
    }

    pub fn dt(&self) -> Option<f32> {
        self.samples.back().map(|&dt| dt as f32)
    }

    pub fn fps(&self) -> f64 {
        if self.samples.is_empty() {
            return 0.0;
        }
        let avg_dt: f64 = self.samples.iter().sum::<f64>() / self.samples.len() as f64;
        1.0 / avg_dt
    }

    pub fn clear_history(&mut self) {
        self.samples.clear();
    }
}
