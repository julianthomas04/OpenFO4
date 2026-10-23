use crate::rendering::hud::quad::PxRect;

/// The largest whole-pixel rectangle of `video`'s aspect ratio that fits `target` (both in
/// pixels), centred; the rest of the target stays black bars. `None` when either size is empty.
pub fn fit_rect(video: (u32, u32), target: (u32, u32)) -> Option<PxRect> {
    if video.0 == 0 || video.1 == 0 || target.0 == 0 || target.1 == 0 {
        return None;
    }
    let scale =
        (f64::from(target.0) / f64::from(video.0)).min(f64::from(target.1) / f64::from(video.1));
    let width = (f64::from(video.0) * scale)
        .round()
        .clamp(1.0, f64::from(target.0)) as u32;
    let height = (f64::from(video.1) * scale)
        .round()
        .clamp(1.0, f64::from(target.1)) as u32;
    Some(PxRect {
        x: ((target.0 - width) / 2) as f32,
        y: ((target.1 - height) / 2) as f32,
        w: width as f32,
        h: height as f32,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    const VIDEO: (u32, u32) = (1920, 1080);

    /// The rectangle as whole pixels: the fit rounds to them, so the casts are exact.
    fn rect(target: (u32, u32)) -> [u32; 4] {
        let r = fit_rect(VIDEO, target).unwrap();
        [r.x as u32, r.y as u32, r.w as u32, r.h as u32]
    }

    #[test]
    fn the_same_aspect_fills_the_target() {
        assert_eq!(rect((1920, 1080)), [0, 0, 1920, 1080]);
        assert_eq!(rect((1280, 720)), [0, 0, 1280, 720]);
        assert_eq!(rect((3840, 2160)), [0, 0, 3840, 2160]);
    }

    #[test]
    fn a_16_10_window_gets_bars_above_and_below() {
        assert_eq!(rect((1920, 1200)), [0, 60, 1920, 1080]);
    }

    #[test]
    fn a_21_9_window_gets_bars_left_and_right() {
        assert_eq!(rect((2560, 1080)), [320, 0, 1920, 1080]);
    }

    #[test]
    fn a_4_3_window_gets_bars_above_and_below() {
        assert_eq!(rect((1024, 768)), [0, 96, 1024, 576]);
    }

    #[test]
    fn a_portrait_window_is_letterboxed_too() {
        assert_eq!(rect((720, 1280)), [0, 437, 720, 405]);
    }

    #[test]
    fn a_target_smaller_than_the_video_scales_down() {
        assert_eq!(rect((640, 480)), [0, 60, 640, 360]);
    }

    #[test]
    fn the_rectangle_never_leaves_the_target_and_is_centred_to_a_pixel() {
        for target in [(1001, 563), (1, 1), (3, 1000), (1000, 3), (777, 1213)] {
            let r = fit_rect(VIDEO, target).unwrap();
            assert!(r.x >= 0.0 && r.y >= 0.0, "{target:?}");
            assert!(r.x + r.w <= target.0 as f32, "{target:?}");
            assert!(r.y + r.h <= target.1 as f32, "{target:?}");
            assert!(r.w >= 1.0 && r.h >= 1.0, "{target:?}");
            assert!(
                (target.0 as f32 - r.w - 2.0 * r.x).abs() <= 1.0
                    && (target.1 as f32 - r.h - 2.0 * r.y).abs() <= 1.0,
                "{target:?}"
            );
        }
    }

    #[test]
    fn an_empty_size_has_no_rectangle() {
        assert!(fit_rect((0, 1080), (1920, 1080)).is_none());
        assert!(fit_rect(VIDEO, (0, 1080)).is_none());
        assert!(fit_rect(VIDEO, (1920, 0)).is_none());
    }
}
