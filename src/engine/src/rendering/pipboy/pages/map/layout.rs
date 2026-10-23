//! The map page's layout keys: each holder's scale range, speeds, gutters, marker scales and
//! mask, the selection tooltip's clips and the hint labels.

use egui::{Pos2, Rect, Vec2};

use super::tooltip::{self, TooltipLayout};
use super::view::{ClampInset, MarkerScales, ModeParams, PanGutters, Rules};
use crate::rendering::pipboy::shapes::{OutlinedShape, rect_of};
use crate::ui_layout::{
    BoundsKey, LiteralKey, ModuleKeys, NumberKey, PlacementKey, ShapeKey, TranslateKey, UiLayout,
    UiLayoutError, key_check_fields,
};

pub(super) const MOVIE: &str = "Pipboy_MapPage";

struct ModeKeys {
    min_scale: NumberKey,
    max_scale: NumberKey,
    start_zoom: NumberKey,
    wheel_speed: NumberKey,
    drag_speed: NumberKey,
    marker_min: NumberKey,
    marker_max: NumberKey,
    marker_selected_min: NumberKey,
    marker_selected_max: NumberKey,
    pan_left: NumberKey,
    pan_right: NumberKey,
    pan_top: NumberKey,
    pan_bottom: NumberKey,
    mask: PlacementKey,
    mask_bounds: BoundsKey,
    window: ShapeKey,
    texture_holder: PlacementKey,
    marker_holder: PlacementKey,
}

key_check_fields!(ModeKeys {
    min_scale,
    max_scale,
    start_zoom,
    wheel_speed,
    drag_speed,
    marker_min,
    marker_max,
    marker_selected_min,
    marker_selected_max,
    pan_left,
    pan_right,
    pan_top,
    pan_bottom,
    mask,
    mask_bounds,
    window,
    texture_holder,
    marker_holder
});

/// The keys of one mode: `$holder` names the holder class whose constructor sets the scale range
/// and speeds, `$markers` the class that supplies the marker scales, `$pan` the one that supplies
/// the pan gutters, `$mask` the mask clip and `$instance` the holder's key-form instance.
macro_rules! mode_keys {
    (holder: $holder:literal, markers: $markers:literal, pan: $pan:literal,
     mask: $mask:literal, instance: $instance:literal) => {
        ModeKeys {
            min_scale: NumberKey::new(MOVIE, concat!($holder, ".MIN_SCALE")),
            max_scale: NumberKey::new(MOVIE, concat!($holder, ".MAX_SCALE")),
            start_zoom: NumberKey::new(MOVIE, concat!($holder, ".START_ZOOM")),
            wheel_speed: NumberKey::new(MOVIE, concat!($holder, ".MOUSE_WHEEL_ZOOM_SPEED")),
            drag_speed: NumberKey::new(MOVIE, concat!($holder, ".MOUSE_DRAG_MOVE_SPEED")),
            marker_min: NumberKey::new(MOVIE, concat!($markers, ".MARKER_SCALE_MIN")),
            marker_max: NumberKey::new(MOVIE, concat!($markers, ".MARKER_SCALE_MAX")),
            marker_selected_min: NumberKey::new(
                MOVIE,
                concat!($markers, ".MARKER_SCALE_SELECTED_MIN"),
            ),
            marker_selected_max: NumberKey::new(
                MOVIE,
                concat!($markers, ".MARKER_SCALE_SELECTED_MAX"),
            ),
            pan_left: NumberKey::new(MOVIE, concat!($pan, ".LEFT_PAN_GUTTER_PCT")),
            pan_right: NumberKey::new(MOVIE, concat!($pan, ".RIGHT_PAN_GUTTER_PCT")),
            pan_top: NumberKey::new(MOVIE, concat!($pan, ".TOP_PAN_GUTTER_PCT")),
            pan_bottom: NumberKey::new(MOVIE, concat!($pan, ".BOTTOM_PAN_GUTTER_PCT")),
            mask: PlacementKey::new(MOVIE, $mask),
            mask_bounds: BoundsKey::new(MOVIE, $mask),
            window: ShapeKey::new(MOVIE, concat!($mask, "/@2")),
            texture_holder: PlacementKey::new(MOVIE, concat!($instance, "/TextureHolder_mc")),
            marker_holder: PlacementKey::new(MOVIE, concat!($instance, "/MarkerHolder_mc")),
        }
    };
}

static WORLD: ModeKeys = mode_keys!(
    holder: "WorldMapHolder",
    markers: "MapHolder",
    pan: "WorldMapHolder",
    mask: "WorldMapMask_mc",
    instance: "[WorldMapHolder]"
);

static LOCAL: ModeKeys = mode_keys!(
    holder: "LocalMapHolder",
    markers: "LocalMapHolder",
    pan: "TextureMapHolder",
    mask: "LocalMapMask_mc",
    instance: "[LocalMapHolder]"
);

struct MapKeys {
    select_radius: NumberKey,
    close_enough: NumberKey,
    border_sector: LiteralKey,
    inset_top: LiteralKey,
    inset_bottom: LiteralKey,
    inset_left: LiteralKey,
    inset_right: LiteralKey,
    fast_travel: TranslateKey,
    place_marker: TranslateKey,
    remove_marker: TranslateKey,
    world_map: TranslateKey,
    local_map: TranslateKey,
}

static KEYS: MapKeys = MapKeys {
    select_radius: NumberKey::new(MOVIE, "MapHolder.SELECT_RADIUS"),
    close_enough: NumberKey::new(MOVIE, "Shared.GlobalFunc.CLOSE_ENOUGH_EPSILON"),
    border_sector: LiteralKey::new(MOVIE, "MapHolder.GetBorderIntersection#Math.PI*", 1),
    inset_top: LiteralKey::new(MOVIE, "MapHolder.PositionMarkers#$3=", 1),
    inset_bottom: LiteralKey::new(MOVIE, "MapHolder.PositionMarkers#$4=", 1),
    inset_left: LiteralKey::new(MOVIE, "MapHolder.PositionMarkers#$5=", 1),
    inset_right: LiteralKey::new(MOVIE, "MapHolder.PositionMarkers#$6=", 1),
    fast_travel: TranslateKey::text("$FastTravel"),
    place_marker: TranslateKey::text("$PLACEMARKER"),
    remove_marker: TranslateKey::text("$RemoveMarker"),
    world_map: TranslateKey::text("$World Map"),
    local_map: TranslateKey::text("$Local Map"),
};

key_check_fields!(MapKeys {
    select_radius,
    close_enough,
    border_sector,
    inset_top,
    inset_bottom,
    inset_left,
    inset_right,
    fast_travel,
    place_marker,
    remove_marker,
    world_map,
    local_map
});

fn validate(layout: &UiLayout) -> Result<(), UiLayoutError> {
    MapLayout::resolve(layout).map(drop)
}

pub static LAYOUT_KEYS: ModuleKeys = ModuleKeys {
    keys: &[&WORLD, &LOCAL, &KEYS, &tooltip::KEYS],
    resolve: validate,
};

pub struct HintLabels {
    pub fast_travel: String,
    pub place_marker: String,
    pub remove_marker: String,
    pub world_map: String,
    pub local_map: String,
}

pub struct MapLayout {
    pub world: ModeParams,
    pub local: ModeParams,
    pub rules: Rules,
    pub tooltip: TooltipLayout,
    pub labels: HintLabels,
}

fn point(layout: &UiLayout, key: &PlacementKey) -> Result<Vec2, UiLayoutError> {
    let matrix = layout.get(key)?.world.matrix;
    Ok(Vec2::new(matrix.tx, matrix.ty))
}

fn number(layout: &UiLayout, key: &NumberKey) -> Result<f32, UiLayoutError> {
    Ok(layout.get(key)? as f32)
}

impl ModeKeys {
    fn resolve(&self, layout: &UiLayout) -> Result<ModeParams, UiLayoutError> {
        let mask_origin = point(layout, &self.mask)?;
        let mask_size = rect_of(layout.get(&self.mask_bounds)?).size();
        Ok(ModeParams {
            min_scale: number(layout, &self.min_scale)?,
            max_scale: number(layout, &self.max_scale)?,
            start_zoom: number(layout, &self.start_zoom)?,
            wheel_speed: number(layout, &self.wheel_speed)?,
            drag_speed: number(layout, &self.drag_speed)?,
            marker_scales: MarkerScales {
                min: number(layout, &self.marker_min)?,
                max: number(layout, &self.marker_max)?,
                selected_min: number(layout, &self.marker_selected_min)?,
                selected_max: number(layout, &self.marker_selected_max)?,
            },
            pan_gutters: PanGutters {
                left: number(layout, &self.pan_left)?,
                right: number(layout, &self.pan_right)?,
                top: number(layout, &self.pan_top)?,
                bottom: number(layout, &self.pan_bottom)?,
            },
            border: Rect::from_min_size(Pos2::new(mask_origin.x, mask_origin.y), mask_size),
            window: OutlinedShape::fill_area(layout, &self.window)?,
            texture_offset: point(layout, &self.texture_holder)?,
            marker_offset: point(layout, &self.marker_holder)?,
        })
    }
}

impl MapLayout {
    pub fn resolve(layout: &UiLayout) -> Result<Self, UiLayoutError> {
        Ok(Self {
            world: WORLD.resolve(layout)?,
            local: LOCAL.resolve(layout)?,
            rules: Rules {
                select_radius: number(layout, &KEYS.select_radius)?,
                close_enough: number(layout, &KEYS.close_enough)?,
                border_sector: layout.lit_number(&KEYS.border_sector, 0)? as f32,
                clamp_inset: ClampInset {
                    top: layout.lit_number(&KEYS.inset_top, 0)? as f32,
                    bottom: layout.lit_number(&KEYS.inset_bottom, 0)? as f32,
                    left: layout.lit_number(&KEYS.inset_left, 0)? as f32,
                    right: layout.lit_number(&KEYS.inset_right, 0)? as f32,
                },
            },
            tooltip: TooltipLayout::resolve(layout)?,
            labels: HintLabels {
                fast_travel: layout.get(&KEYS.fast_travel)?,
                place_marker: layout.get(&KEYS.place_marker)?,
                remove_marker: layout.get(&KEYS.remove_marker)?,
                world_map: layout.get(&KEYS.world_map)?,
                local_map: layout.get(&KEYS.local_map)?,
            },
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    #[ignore = "needs OPENFO4_INSTALL_DIR with the installer's layout documents"]
    fn the_installed_clamp_insets_come_from_the_marker_placement_code() {
        let installed = crate::ui_layout::test_support::installed_layout();
        let rules = MapLayout::resolve(&installed).unwrap().rules;
        let inset = rules.clamp_inset;
        assert!(inset.top > 0.0 && inset.right > 0.0);
        assert!(inset.bottom >= 0.0 && inset.left >= 0.0);
    }

    #[test]
    #[ignore = "needs OPENFO4_INSTALL_DIR with the installer's layout documents"]
    fn the_installed_clip_window_is_the_masks_fill_without_its_stroke() {
        let installed = crate::ui_layout::test_support::installed_layout();
        let layout = MapLayout::resolve(&installed).unwrap();
        for (params, keys) in [(&layout.world, &WORLD), (&layout.local, &LOCAL)] {
            let fill =
                crate::rendering::pipboy::shapes::edge_rect(&installed, &keys.window).unwrap();
            crate::ui_layout::test_support::assert_all_close(
                &[
                    params.window.min.x,
                    params.window.min.y,
                    params.window.max.x,
                    params.window.max.y,
                ],
                &[fill.min.x, fill.min.y, fill.max.x, fill.max.y],
            );
            assert!(params.border.contains_rect(params.window));
        }
    }
}
