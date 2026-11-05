//! Proof that the reveal fade shows no mesh with a stand-in texture: while the world fades in,
//! every frame counts the drawn cells and skinned meshes that use a texture which is not
//! loaded and bound yet, and the end of the fade reports the tally.

use std::collections::BTreeSet;
use std::fmt;

/// The most drawn things a report names; the rest are counted.
const LISTED_DRAWS: usize = 20;

/// Something the world draws with its own set of textures.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord)]
pub enum Draw {
    FullCell((i32, i32)),
    LodCell((i32, i32)),
    /// A character or animated object, by its rig name.
    Skinned(String),
}

impl fmt::Display for Draw {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::FullCell((x, y)) => write!(f, "full ({x}, {y})"),
            Self::LodCell((x, y)) => write!(f, "LOD ({x}, {y})"),
            Self::Skinned(name) => write!(f, "skinned '{name}'"),
        }
    }
}

#[derive(Debug, Default)]
struct Tally {
    frames: u32,
    worst: usize,
    draws: BTreeSet<Draw>,
}

#[derive(Debug, Default)]
pub struct RevealTextureCheck {
    fade: Option<Tally>,
}

impl RevealTextureCheck {
    /// Takes one frame. While `fading`, `unready_draws` lists what this frame draws with a
    /// texture that is not ready; it is not asked otherwise. The first frame after a fade
    /// returns the fade's report.
    pub fn frame(
        &mut self,
        fading: bool,
        unready_draws: impl FnOnce() -> Vec<Draw>,
    ) -> Option<RevealTextureReport> {
        if !fading {
            return self.fade.take().map(RevealTextureReport::from);
        }
        let draws = unready_draws();
        let tally = self.fade.get_or_insert_with(Tally::default);
        tally.frames += 1;
        tally.worst = tally.worst.max(draws.len());
        tally.draws.extend(draws);
        None
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RevealTextureReport {
    pub frames: u32,
    /// The most draws with an unready texture any one frame of the fade showed.
    pub worst_unready_draws: usize,
    /// Every draw that showed an unready texture on some frame of the fade.
    pub draws: Vec<Draw>,
}

impl RevealTextureReport {
    pub fn is_clean(&self) -> bool {
        self.worst_unready_draws == 0
    }
}

impl From<Tally> for RevealTextureReport {
    fn from(tally: Tally) -> Self {
        Self {
            frames: tally.frames,
            worst_unready_draws: tally.worst,
            draws: tally.draws.into_iter().collect(),
        }
    }
}

impl fmt::Display for RevealTextureReport {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            f,
            "frames={} worst_unready_draws={} draws=[",
            self.frames, self.worst_unready_draws
        )?;
        for (index, draw) in self.draws.iter().take(LISTED_DRAWS).enumerate() {
            if index > 0 {
                f.write_str(", ")?;
            }
            write!(f, "{draw}")?;
        }
        let unlisted = self.draws.len().saturating_sub(LISTED_DRAWS);
        if unlisted > 0 {
            write!(f, ", and {unlisted} more")?;
        }
        f.write_str("]")
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn fade(check: &mut RevealTextureCheck, frames: Vec<Vec<Draw>>) -> Option<RevealTextureReport> {
        for draws in frames {
            assert_eq!(check.frame(true, || draws), None);
        }
        check.frame(false, || unreachable!("asked only while fading"))
    }

    #[test]
    fn nothing_is_reported_without_a_fade() {
        let mut check = RevealTextureCheck::default();
        assert_eq!(check.frame(false, || unreachable!()), None);
    }

    #[test]
    fn a_fade_without_unready_draws_is_clean() {
        let mut check = RevealTextureCheck::default();

        let report = fade(&mut check, vec![Vec::new(), Vec::new()]).expect("the fade ended");

        assert!(report.is_clean());
        assert_eq!(
            report.to_string(),
            "frames=2 worst_unready_draws=0 draws=[]"
        );
    }

    #[test]
    fn the_report_holds_the_worst_frame_and_every_draw_once() {
        let mut check = RevealTextureCheck::default();
        let frames = vec![
            vec![Draw::LodCell((1, 2))],
            vec![Draw::Skinned("door".to_owned()), Draw::LodCell((1, 2))],
            Vec::new(),
        ];

        let report = fade(&mut check, frames).expect("the fade ended");

        assert!(!report.is_clean());
        assert_eq!(
            report.to_string(),
            "frames=3 worst_unready_draws=2 draws=[LOD (1, 2), skinned 'door']"
        );
    }

    #[test]
    fn each_fade_is_reported_on_its_own() {
        let mut check = RevealTextureCheck::default();
        fade(&mut check, vec![vec![Draw::FullCell((0, 0))]]);

        let second = fade(&mut check, vec![Vec::new()]).expect("the second fade ended");

        assert_eq!(second.frames, 1);
        assert!(second.is_clean());
    }

    #[test]
    fn a_long_list_names_the_first_draws_and_counts_the_rest() {
        let mut check = RevealTextureCheck::default();
        let draws = (0..LISTED_DRAWS as i32 + 3)
            .map(|x| Draw::FullCell((x, 0)))
            .collect();

        let report = fade(&mut check, vec![draws]).expect("the fade ended");

        assert!(report.to_string().ends_with("full (19, 0), and 3 more]"));
    }
}
