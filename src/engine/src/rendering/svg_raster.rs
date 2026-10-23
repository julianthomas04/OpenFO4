//! Rendering a parsed SVG into a premultiplied RGBA pixmap of a given pixel size, shared by every
//! layer that turns exported vector art into pixels.

use resvg::{tiny_skia, usvg};

/// The largest raster edge in pixels; a bigger image is a data error, not a texture.
pub const MAX_RASTER_EDGE: u32 = 8192;

#[derive(Debug, Clone, Copy, PartialEq, Eq, thiserror::Error)]
#[error("art would rasterise to {width}x{height} pixels")]
pub struct BadRasterSize {
    pub width: u32,
    pub height: u32,
}

/// Whether a `width` x `height` raster is a size art can be rendered at.
pub fn check_size(width: u32, height: u32) -> Result<(), BadRasterSize> {
    if width == 0 || height == 0 || width > MAX_RASTER_EDGE || height > MAX_RASTER_EDGE {
        Err(BadRasterSize { width, height })
    } else {
        Ok(())
    }
}

/// `tree` stretched over a new `width` x `height` pixmap.
pub fn render(
    tree: &usvg::Tree,
    width: u32,
    height: u32,
) -> Result<tiny_skia::Pixmap, BadRasterSize> {
    check_size(width, height)?;
    let mut pixmap =
        tiny_skia::Pixmap::new(width, height).ok_or(BadRasterSize { width, height })?;
    let transform = tiny_skia::Transform::from_scale(
        width as f32 / tree.size().width(),
        height as f32 / tree.size().height(),
    );
    resvg::render(tree, transform, &mut pixmap.as_mut());
    Ok(pixmap)
}

#[cfg(test)]
mod tests {
    use super::*;

    const LEFT_HALF_SVG: &str = r#"<svg xmlns="http://www.w3.org/2000/svg" width="20" height="10" viewBox="0 0 20 10"><rect width="10" height="10" fill="white"/></svg>"#;

    fn tree() -> usvg::Tree {
        usvg::Tree::from_data(LEFT_HALF_SVG.as_bytes(), &usvg::Options::default()).unwrap()
    }

    #[test]
    fn the_art_is_stretched_over_the_whole_pixmap() {
        let pixmap = render(&tree(), 40, 20).unwrap();
        assert_eq!((pixmap.width(), pixmap.height()), (40, 20));
        let alpha = |x: u32, y: u32| pixmap.pixel(x, y).unwrap().alpha();
        assert_eq!(alpha(5, 10), 255);
        assert_eq!(alpha(35, 10), 0);
    }

    #[test]
    fn a_degenerate_or_oversized_pixmap_is_rejected() {
        for (width, height) in [(0, 5), (5, 0), (MAX_RASTER_EDGE + 1, 5)] {
            assert_eq!(
                render(&tree(), width, height).unwrap_err(),
                BadRasterSize { width, height }
            );
        }
    }
}
