//! The rank stars, the SPECIAL counter stars, the blink and the multiplied overlay.

use egui::Vec2;

use super::{single, unresolved};
use crate::ui_layout::{
    KeyGroup, PlacementKey, ShapeKey, StageKey, SymbolTimelineKey, TimelineKey, UiLayout,
    UiLayoutError,
};

literal!(GRID_STAR_PITCH, "PerkGrid.PopulateGrid#*uirank", 1);
literal!(GRID_STAR_SCALE, "PerkGrid.PopulateGrid#starClip.scaleX=", 1);
literal!(
    SPECIAL_STAR_PITCH,
    "SPECIALStarHolder.redrawUIComponent#*$1",
    1
);
literal!(
    SPECIAL_STAR_HALVES,
    "SPECIALStarHolder.redrawUIComponent#width/",
    1
);
literal!(
    SPECIAL_STAR_CAP,
    "SPECIALStarHolder.set:value#Math.min(1)",
    1
);

pub(crate) static OVERLAY: PlacementKey = PlacementKey::new(super::MOVIE, "[PerkGrid]/Overlay_mc");
pub(crate) static BLINK_FADE: PlacementKey =
    PlacementKey::new(super::MOVIE, "[PerkRankStar]:Available/@1/@3");
pub(crate) static BLINK_TIMELINE: TimelineKey =
    TimelineKey::new(super::MOVIE, "[PerkRankStar]:Available/@1");
pub(crate) static MOVIE_STAGE: StageKey = StageKey::of(super::MOVIE);
pub(crate) static SPECIAL_STAR_SHAPE: ShapeKey = ShapeKey::new(super::MOVIE, "[SPECIALStar]/@1");
pub(crate) static RANK_STAR_SHAPE: ShapeKey =
    ShapeKey::new(super::MOVIE, "[PerkRankStar]:Empty/@1/@1");
/// The rank star's symbol; its labels name the frames of the three states.
pub(crate) static RANK_STAR_SYMBOL: SymbolTimelineKey =
    SymbolTimelineKey::new(super::MOVIE, "PerkRankStar");

pub(crate) static KEYS: KeyGroup = KeyGroup(&[
    &GRID_STAR_PITCH,
    &GRID_STAR_SCALE,
    &SPECIAL_STAR_PITCH,
    &SPECIAL_STAR_HALVES,
    &SPECIAL_STAR_CAP,
    &OVERLAY,
    &BLINK_FADE,
    &BLINK_TIMELINE,
    &MOVIE_STAGE,
    &SPECIAL_STAR_SHAPE,
    &RANK_STAR_SHAPE,
    &RANK_STAR_SYMBOL,
]);

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct GridStars {
    pub pitch: f32,
    pub scale: f32,
    pub size: Vec2,
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct SpecialStars {
    pub pitch: f32,
    pub cap: u8,
    pub size: Vec2,
    pub halves: f32,
}

/// The frames of the rank star clip, 0-based, per state.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct StarFrames {
    pub empty: usize,
    pub full: usize,
}

/// The blinking rank star: the full star's opacity per frame laid over the empty star.
pub struct StarBlink {
    pub fade: Vec<f32>,
    pub fps: f32,
}

pub struct OverlayLayout {
    pub alpha: f32,
}

pub struct StarsLayout {
    pub overlay: OverlayLayout,
    pub grid: GridStars,
    pub special: SpecialStars,
    pub rank_star_size: Vec2,
    pub frames: StarFrames,
    pub blink: StarBlink,
}

/// The extent of a shape's bounds, the size the star is drawn at before any scale.
fn extent(bounds: [f32; 4]) -> Vec2 {
    Vec2::new(bounds[2] - bounds[0], bounds[3] - bounds[1])
}

fn star_frames(layout: &UiLayout) -> Result<StarFrames, UiLayoutError> {
    let labels = layout.get(&RANK_STAR_SYMBOL)?.labels;
    let frame = |label: &str| {
        labels
            .get(label)
            .and_then(|frame| usize::try_from(*frame).ok())
            .and_then(|frame| frame.checked_sub(1))
            .ok_or_else(|| {
                unresolved(
                    RANK_STAR_SYMBOL.path,
                    format!("the rank star has no frame labelled '{label}'"),
                )
            })
    };
    Ok(StarFrames {
        empty: frame("Empty")?,
        full: frame("Full")?,
    })
}

pub fn load(layout: &UiLayout) -> Result<StarsLayout, UiLayoutError> {
    let overlay = layout.get(&OVERLAY)?;
    if overlay.local.blend.as_deref() != Some("multiply") {
        return Err(unresolved(
            OVERLAY.path,
            "the overlay is not blended by multiplication",
        ));
    }
    let rank_star = layout.get(&RANK_STAR_SHAPE)?;
    let rank_star_size = extent(rank_star.bounds);
    let fade_placement = layout.get(&BLINK_FADE)?;
    let blink_frames = layout.get(&BLINK_TIMELINE)?.frame_count;
    let fade = (1..=blink_frames)
        .map(|frame| {
            fade_placement
                .track()
                .iter()
                .rev()
                .find(|key| key.frame <= frame)
                .map_or(fade_placement.local.color.mult[3], |key| {
                    key.placement.color.mult[3]
                })
        })
        .collect();
    let special_star = layout.get(&SPECIAL_STAR_SHAPE)?;
    let special_matrix = special_star.placement.local.matrix;
    let special_extent = extent(special_star.bounds);
    Ok(StarsLayout {
        overlay: OverlayLayout {
            alpha: overlay.local.color.mult[3],
        },
        grid: GridStars {
            pitch: single(layout, &GRID_STAR_PITCH)?,
            scale: single(layout, &GRID_STAR_SCALE)?,
            size: rank_star_size,
        },
        special: SpecialStars {
            pitch: single(layout, &SPECIAL_STAR_PITCH)?,
            cap: single(layout, &SPECIAL_STAR_CAP)? as u8,
            size: Vec2::new(
                special_matrix.a * special_extent.x,
                special_matrix.d * special_extent.y,
            ),
            halves: single(layout, &SPECIAL_STAR_HALVES)?,
        },
        rank_star_size,
        frames: star_frames(layout)?,
        blink: StarBlink {
            fade,
            fps: layout.get(&MOVIE_STAGE)?.fps,
        },
    })
}

#[cfg(test)]
#[allow(clippy::float_cmp, reason = "exact literals")]
mod tests {
    use super::*;
    use crate::rendering::pipboy::test_layout::pipboy_layout;

    #[test]
    fn a_stars_size_is_the_extent_of_its_bounds_not_the_far_corner() {
        assert_eq!(extent([10.0, 20.0, 15.0, 32.0]), Vec2::new(5.0, 12.0));
    }

    #[test]
    fn the_rank_star_frames_follow_the_symbols_labels() {
        let frames = load(&pipboy_layout()).unwrap().frames;
        assert_ne!(frames.empty, frames.full);
    }

    #[test]
    fn the_blink_has_one_opacity_per_frame_of_the_timeline() {
        let layout = pipboy_layout();
        let blink = load(&layout).unwrap().blink;
        let frames = layout.get(&BLINK_TIMELINE).unwrap().frame_count;
        assert_eq!(blink.fade.len(), frames as usize);
        assert!(blink.fade.iter().all(|alpha| (0.0..=1.0).contains(alpha)));
        assert_eq!(blink.fps, layout.get(&MOVIE_STAGE).unwrap().fps);
    }

    #[test]
    fn the_overlay_is_multiplied_at_its_authored_alpha() {
        let layout = pipboy_layout();
        let alpha = load(&layout).unwrap().overlay.alpha;
        assert_eq!(alpha, layout.get(&OVERLAY).unwrap().local.color.mult[3]);
    }

    #[test]
    fn a_movie_that_does_not_multiply_its_overlay_is_rejected() {
        let movie = crate::rendering::pipboy::pages::perk_chart::fixture::level_up_menu_movie()
            .replace("multiply", "normal");
        let layout = crate::ui_layout::test_support::layout_with_movie(super::super::MOVIE, &movie);
        let error = load(&layout).err().expect("the load fails");
        assert!(error.to_string().contains("multiplication"), "{error}");
    }
}
