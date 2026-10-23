//! The crosshair, driven entirely by the game's own animation data. The converted `HUDMenu` movie
//! holds the reticle clips as sprites with per-frame placements; they are flattened once at start
//! into rectangles per frame ([`super::clip`]) and replayed here. Nothing is re-tweened: the dot,
//! the Activate reticle and the dot<->Activate morph are exactly the frames the game authored.
//!
//! Unarmed shows the **Dot**; aiming at an interactable plays the dot-to-activate clip to the
//! Activate reticle and the activate-to-dot clip back. The armed Standard (procedural spread)
//! reticle returns with weapons, a separate mechanism.

use super::clip::{ClipRect, flatten_clip};
use super::layout::Layout;
use super::quad::{PxRect, QuadBuilder};
use crate::sim::CrosshairState;
use crate::ui_layout::{KeyGroup, Movie, TimelineKey, UiLayout, UiLayoutError};

/// Clamp animation catch-up after a hitch, like `SimWorld::tick`.
const MAX_ANIM_CATCHUP: f32 = 0.25;

const MOVIE: &str = "HUDMenu";
const CLIPS: &str = "CenterGroup_mc/HUDCrosshair_mc/CrosshairBase_mc/CrosshairClips_mc";
const DOT_LOOP: TimelineKey = TimelineKey::new(
    MOVIE,
    "CenterGroup_mc/HUDCrosshair_mc/CrosshairBase_mc/CrosshairClips_mc/Dot_Dot",
);
const DOT_TO_ACTIVATE: TimelineKey = TimelineKey::new(
    MOVIE,
    "CenterGroup_mc/HUDCrosshair_mc/CrosshairBase_mc/CrosshairClips_mc/Dot_Activate",
);
const ACTIVATE_TO_DOT: TimelineKey = TimelineKey::new(
    MOVIE,
    "CenterGroup_mc/HUDCrosshair_mc/CrosshairBase_mc/CrosshairClips_mc/Activate_Dot",
);

/// The crosshair keys, checked at boot.
pub static KEYS: KeyGroup = KeyGroup(&[&DOT_LOOP, &DOT_TO_ACTIVATE, &ACTIVATE_TO_DOT]);

type Frame = Vec<ClipRect>;

/// The reticle's animation as the movie authors it.
pub struct CrosshairSpec {
    frame_rate: f32,
    dot_loop: Vec<Frame>,
    dot_to_activate: Vec<Frame>,
    activate_to_dot: Vec<Frame>,
}

impl CrosshairSpec {
    pub fn resolve(layout: &UiLayout) -> Result<Self, UiLayoutError> {
        let movie = layout.movie(MOVIE)?;
        // A non-finite rate would make the fixed-step loop in `update` spin forever.
        let frame_rate = movie.stage().fps;
        if !(frame_rate.is_finite() && frame_rate > 0.0) {
            return Err(UiLayoutError::unresolved(
                MOVIE,
                CLIPS,
                format!("the frame rate {frame_rate} is not positive"),
            ));
        }
        Ok(Self {
            frame_rate,
            dot_loop: flatten(layout, &movie, &DOT_LOOP)?,
            dot_to_activate: flatten(layout, &movie, &DOT_TO_ACTIVATE)?,
            activate_to_dot: flatten(layout, &movie, &ACTIVATE_TO_DOT)?,
        })
    }
}

/// The frames of a clip; every clip must show something, since the resting states hold a frame
/// and the transitions index frames during playback.
fn flatten(
    layout: &UiLayout,
    movie: &Movie,
    key: &TimelineKey,
) -> Result<Vec<Frame>, UiLayoutError> {
    let frame_count = layout.get(key)?.frame_count;
    let frames = flatten_clip(movie, MOVIE, key.path, frame_count)
        .map_err(|error| error.into_layout(MOVIE, key.path))?;
    if frames.iter().all(Vec::is_empty) {
        return Err(UiLayoutError::unresolved(
            MOVIE,
            key.path,
            "the clip shows nothing in any frame",
        ));
    }
    Ok(frames)
}

#[derive(Copy, Clone, PartialEq, Eq)]
enum Mode {
    Hidden,
    Dot,
    Activate,
}

impl Mode {
    fn of(state: CrosshairState) -> Self {
        match state {
            CrosshairState::Hidden => Mode::Hidden,
            // Dot, plus the armed Standard which has no keyframe clip yet (no
            // weapons): fall back to the dot reticle.
            CrosshairState::Dot | CrosshairState::Standard { .. } => Mode::Dot,
            CrosshairState::Activate => Mode::Activate,
        }
    }
}

#[derive(Copy, Clone)]
enum Clip {
    Dot2Activate,
    Activate2Dot,
}

#[derive(Copy, Clone)]
struct Playing {
    clip: Clip,
    frame: usize,
    target: Mode,
}

pub struct CrosshairWidget {
    step_dt: f32, // 1 / frame_rate
    dot2activate: Vec<Frame>,
    activate2dot: Vec<Frame>,
    /// Held (static) frames for the resting states.
    held_dot: Frame,
    held_activate: Frame,
    /// First non-empty frame of each transition (the clips open on a blank frame).
    dot2activate_start: usize,
    activate2dot_start: usize,
    mode: Mode,
    playing: Option<Playing>,
    accum: f32,
}

impl CrosshairWidget {
    pub fn new(spec: CrosshairSpec) -> Self {
        // The resting reticles are a single held frame, not a replayed loop: the resting clips
        // are static holds (the settled Activate is the last frame of the transition into it),
        // and replaying the dot loop would blink, since its first frame is the empty
        // pre-placement frame. Only the dot<->Activate transitions animate.
        let held_dot = last_non_empty(&spec.dot_loop);
        let held_activate = last_non_empty(&spec.dot_to_activate);
        let dot2activate_start = first_non_empty(&spec.dot_to_activate);
        let activate2dot_start = first_non_empty(&spec.activate_to_dot);
        Self {
            step_dt: 1.0 / spec.frame_rate,
            dot2activate: spec.dot_to_activate,
            activate2dot: spec.activate_to_dot,
            held_dot,
            held_activate,
            dot2activate_start,
            activate2dot_start,
            mode: Mode::Hidden,
            playing: None,
            accum: 0.0,
        }
    }

    /// Advance the reticle toward `requested` at the clips' authored frame rate.
    pub fn update(&mut self, requested: CrosshairState, dt: f32) {
        let req = Mode::of(requested);
        self.accum = (self.accum + dt.max(0.0)).min(MAX_ANIM_CATCHUP);
        while self.accum >= self.step_dt {
            self.accum -= self.step_dt;
            self.step(req);
        }
    }

    fn step(&mut self, req: Mode) {
        if let Some(p) = self.playing {
            let next = p.frame + 1;
            let len = self.clip(p.clip).len();
            if next >= len {
                self.playing = None;
                self.mode = p.target;
            } else {
                self.playing = Some(Playing { frame: next, ..p });
            }
            return;
        }
        if req == self.mode {
            return;
        }
        match (self.mode, req) {
            (Mode::Dot, Mode::Activate) => {
                self.playing = Some(Playing {
                    clip: Clip::Dot2Activate,
                    frame: self.dot2activate_start,
                    target: Mode::Activate,
                });
            }
            (Mode::Activate, Mode::Dot) => {
                self.playing = Some(Playing {
                    clip: Clip::Activate2Dot,
                    frame: self.activate2dot_start,
                    target: Mode::Dot,
                });
            }
            // Transitions involving Hidden (spawn / HUD show-hide) are instant;
            // FO4's none2* clips aren't needed while the HUD gates on spawn.
            _ => self.mode = req,
        }
    }

    fn clip(&self, c: Clip) -> &[Frame] {
        match c {
            Clip::Dot2Activate => &self.dot2activate,
            Clip::Activate2Dot => &self.activate2dot,
        }
    }

    pub fn emit(&self, qb: &mut QuadBuilder, layout: &Layout) {
        let (cx, cy) = layout.center();
        let frame: &Frame = match self.playing {
            Some(p) => &self.clip(p.clip)[p.frame],
            None => match self.mode {
                Mode::Dot => &self.held_dot,
                Mode::Activate => &self.held_activate,
                Mode::Hidden => return,
            },
        };
        for r in frame {
            qb.rect(
                PxRect {
                    x: cx + layout.s(r.x),
                    y: cy + layout.s(r.y),
                    w: layout.s(r.w),
                    h: layout.s(r.h),
                },
                [r.colour[0], r.colour[1], r.colour[2], r.alpha],
            );
        }
    }
}

fn first_non_empty(frames: &[Frame]) -> usize {
    frames.iter().position(|f| !f.is_empty()).unwrap_or(0)
}

fn last_non_empty(frames: &[Frame]) -> Frame {
    frames
        .iter()
        .rev()
        .find(|f| !f.is_empty())
        .cloned()
        .unwrap_or_default()
}
