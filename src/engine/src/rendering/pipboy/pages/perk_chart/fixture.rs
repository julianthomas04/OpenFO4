//! A small `LevelUpMenu` movie document for the tests, in the layout format 1: the elements and
//! numbers the chart reads, with values shaped like the game's but written here. It names its
//! elements through the chart's own keys, so a key that changes breaks the fixture, not the tests.

use serde_json::{Map, Value, json};

use super::{filter, layout};
use crate::ui_layout::test_support::{
    FIXTURE_BOLD_CLASS, FIXTURE_REGULAR_CLASS, identity_color, set_text_parts, text_character,
};
use crate::ui_layout::{LiteralKey, NumberKey};

pub const CELL_ORIGIN: (f32, f32) = (300.0, 300.0);
pub const CELL_PITCH: (f32, f32) = (250.0, 200.0);
pub const BORDER_SIZE: (f32, f32) = (200.0, 150.0);

const BORDER_OFFSET: (f32, f32) = (-70.0, -10.0);
const CELL_SCALE: f32 = 0.5;
const PANEL: (f32, f32) = (-10.0, 500.0);
const HEADER: (f32, f32) = (340.0, 230.0);

#[derive(Default)]
struct Doc {
    characters: Map<String, Value>,
    instances: Map<String, Value>,
    constants: Map<String, Value>,
    literals: Map<String, Value>,
    symbols: Map<String, Value>,
}

fn matrix(scale: (f32, f32), at: (f32, f32)) -> Value {
    json!([scale.0, 0, 0, scale.1, at.0, at.1])
}

impl Doc {
    fn character(&mut self, character: Value) -> usize {
        let id = self.characters.len() + 1;
        self.characters.insert(id.to_string(), character);
        id
    }

    fn shape(&mut self, bounds: [f32; 4], fills: &Value, lines: &Value) -> usize {
        self.character(json!({
            "kind": "shape", "bounds": bounds, "edgeBounds": bounds, "fills": fills, "lines": lines, "paths": [],
        }))
    }

    fn stroked_shape(
        &mut self,
        bounds: [f32; 4],
        edge: [f32; 4],
        fills: &Value,
        lines: &Value,
    ) -> usize {
        self.character(json!({
            "kind": "shape", "bounds": bounds, "edgeBounds": edge, "fills": fills, "lines": lines, "paths": [],
        }))
    }

    fn sprite(&mut self, frames: u32) -> usize {
        self.character(json!({"kind": "sprite", "frameCount": frames, "labels": {}}))
    }

    fn classed_sprite(&mut self, frames: u32, class: &str) -> usize {
        self.character(
            json!({"kind": "sprite", "class": class, "frameCount": frames, "labels": {}}),
        )
    }

    #[allow(clippy::too_many_arguments, reason = "a fixture builder")]
    fn put(
        &mut self,
        path: &str,
        character: usize,
        kind: &str,
        local: Value,
        world: Value,
        extra: &Value,
    ) {
        let mut instance = json!({
            "char": character, "kind": kind, "depth": 1, "name": "x", "frames": [1, 1],
            "local": {"color": identity_color(), "visible": true},
            "world": {"color": identity_color()},
        });
        instance["local"]["matrix"] = local;
        instance["world"]["matrix"] = world;
        for (key, value) in extra.as_object().into_iter().flatten() {
            if key == "track" {
                instance["track"] = value.clone();
            } else {
                instance["local"][key] = value.clone();
            }
        }
        self.instances.insert(path.to_owned(), instance);
    }

    fn sprite_at(&mut self, path: &str, local: (f32, f32), world: (f32, f32)) {
        let id = self.sprite(1);
        self.put(
            path,
            id,
            "sprite",
            matrix((1.0, 1.0), local),
            matrix((1.0, 1.0), world),
            &json!({}),
        );
    }

    fn text_at(
        &mut self,
        path: &str,
        size: u32,
        bold: bool,
        wrap: bool,
        local: (f32, f32),
        world: (f32, f32),
    ) {
        let mut text = text_character([-2.0, -2.0, 220.0, 33.0]);
        text["size"] = json!(size);
        text["font"] = json!(if bold {
            FIXTURE_BOLD_CLASS
        } else {
            FIXTURE_REGULAR_CLASS
        });
        text["align"] = json!("center");
        text["flags"] =
            json!({"wordWrap": wrap, "multiline": wrap, "autoSize": false, "html": false});
        let id = self.character(text);
        self.put(
            path,
            id,
            "text",
            matrix((1.0, 1.0), local),
            matrix((1.0, 1.0), world),
            &json!({}),
        );
    }

    fn number(&mut self, key: &NumberKey, value: f64) {
        self.constants.insert(key.path.to_owned(), json!(value));
    }

    fn literal(&mut self, key: &LiteralKey, values: &Value) {
        self.literals
            .insert(key.path.to_owned(), json!({ "values": values }));
    }
}

fn grid(doc: &mut Doc) {
    for (row, (cells, borders)) in layout::grid::CELL_PLACEMENTS
        .iter()
        .zip(&layout::grid::CELL_BORDERS)
        .enumerate()
    {
        for (column, (cell, border)) in cells.iter().zip(borders).enumerate() {
            let x = CELL_ORIGIN.0 + column as f32 * CELL_PITCH.0;
            let y = CELL_ORIGIN.1 + row as f32 * CELL_PITCH.1;
            let card = if row == 0 {
                doc.classed_sprite(1, &format!("CardClass{column}"))
            } else {
                doc.sprite(1)
            };
            doc.put(
                cell.path,
                card,
                "sprite",
                matrix((CELL_SCALE, CELL_SCALE), (x, y)),
                matrix((CELL_SCALE, CELL_SCALE), (x, y)),
                &json!({}),
            );
            let hit = doc.shape(
                [0.0, 0.0, BORDER_SIZE.0, BORDER_SIZE.1],
                &json!([]),
                &json!([]),
            );
            let at = (x + BORDER_OFFSET.0, y + BORDER_OFFSET.1);
            doc.put(
                border.path,
                hit,
                "shape",
                matrix((1.0, 1.0), (0.0, 0.0)),
                matrix((1.0, 1.0), at),
                &json!({}),
            );
        }
    }
    for (row, holders) in layout::grid::STAR_HOLDERS.iter().enumerate() {
        for (column, holder) in holders.iter().enumerate() {
            let at = (
                CELL_ORIGIN.0 + column as f32 * CELL_PITCH.0 - 60.0,
                CELL_ORIGIN.1 + (row + 1) as f32 * CELL_PITCH.1 + 100.0,
            );
            let id = doc.sprite(1);
            doc.put(
                holder.path,
                id,
                "sprite",
                matrix((0.55, 0.55), at),
                matrix((0.55, 0.55), at),
                &json!({}),
            );
        }
    }
}

fn headers(doc: &mut Doc) {
    for column in 0..layout::headers::TEXTS.len() {
        let local = (-50.0 + 235.0 * column as f32, -3.0);
        let world = (HEADER.0 + local.0, HEADER.1 + local.1);
        doc.text_at(
            layout::headers::TEXTS[column].path,
            26,
            true,
            false,
            local,
            world,
        );
        doc.text_at(
            layout::headers::SHADOWS[column].path,
            26,
            true,
            false,
            local,
            world,
        );
        let stars = (57.5 + 235.0 * column as f32, 25.0);
        let id = doc.sprite(1);
        doc.put(
            layout::headers::STARS[column].path,
            id,
            "sprite",
            matrix((1.0, 1.0), stars),
            matrix((1.0, 1.0), (HEADER.0 + stars.0, HEADER.1 + stars.1)),
            &json!({}),
        );
    }
    let overlay = doc.sprite(1);
    doc.put(
        layout::stars::OVERLAY.path,
        overlay,
        "sprite",
        matrix((1.0, 1.0), (0.0, 0.0)),
        matrix((1.0, 1.0), (0.0, 0.0)),
        &json!({"blend": "multiply", "color": {"mult": [1, 1, 1, 0.5], "add": [0, 0, 0, 0]}}),
    );
}

fn stars(doc: &mut Doc) {
    let white = json!([{"type": "solid", "color": [255, 255, 255, 255]}]);
    let black_and_yellow = json!([
        {"type": "solid", "color": [0, 0, 0, 255]},
        {"type": "solid", "color": [200, 200, 200, 255]},
    ]);
    let empty = doc.shape([0.0, 0.0, 40.0, 36.0], &white, &json!([]));
    doc.put(
        layout::stars::RANK_STAR_SHAPE.path,
        empty,
        "shape",
        matrix((1.0, 1.0), (0.0, 0.0)),
        matrix((1.0, 1.0), (0.0, 0.0)),
        &json!({}),
    );
    let rank_star = doc.character(json!({
        "kind": "sprite", "class": "PerkRankStar", "frameCount": 3,
        "labels": {"Empty": 1, "Full": 2, "Available": 3},
    }));
    doc.symbols
        .insert("PerkRankStar".to_owned(), json!(rank_star));
    let blink = doc.sprite(40);
    doc.put(
        layout::stars::BLINK_TIMELINE.path,
        blink,
        "sprite",
        matrix((1.0, 1.0), (0.0, 0.0)),
        matrix((1.0, 1.0), (0.0, 0.0)),
        &json!({}),
    );
    let fade = doc.sprite(1);
    let key = |frame: u32, alpha: f32| {
        json!({
            "frame": frame, "matrix": [1, 0, 0, 1, 0, 0],
            "color": {"mult": [1, 1, 1, alpha], "add": [0, 0, 0, 0]}, "visible": true,
        })
    };
    doc.put(
        layout::stars::BLINK_FADE.path,
        fade,
        "sprite",
        matrix((1.0, 1.0), (0.0, 0.0)),
        matrix((1.0, 1.0), (0.0, 0.0)),
        &json!({"track": [key(1, 0.0), key(20, 1.0)]}),
    );
    let special = doc.shape([0.0, 0.0, 40.0, 36.0], &black_and_yellow, &json!([]));
    doc.put(
        layout::stars::SPECIAL_STAR_SHAPE.path,
        special,
        "shape",
        matrix((0.5, 0.5), (0.0, 0.0)),
        matrix((0.5, 0.5), (0.0, 0.0)),
        &json!({}),
    );
}

fn panels(doc: &mut Doc) {
    let at = |x: f32, y: f32| (PANEL.0 + x, PANEL.1 + y);
    doc.sprite_at(layout::info::PANEL.path, PANEL, PANEL);
    doc.sprite_at(
        layout::info::BACKGROUND.path,
        (115.0, 10.0),
        at(115.0, 10.0),
    );
    let background = doc.stroked_shape(
        [-1.0, -1.0, 1000.0, 100.0],
        [0.0, 0.0, 999.0, 99.0],
        &json!([{"type": "solid", "color": [40, 40, 40, 180]}]),
        &json!([{"width": 2, "color": [255, 255, 255, 255]}]),
    );
    doc.put(
        layout::info::BACKGROUND_SHAPE.path,
        background,
        "shape",
        matrix((1.0, 1.0), (0.0, 0.0)),
        matrix((1.0, 1.0), at(115.0, 10.0)),
        &json!({}),
    );
    doc.text_at(
        layout::info::NAME.path,
        26,
        true,
        false,
        (270.0, 15.0),
        at(270.0, 15.0),
    );
    doc.text_at(
        layout::info::DESCRIPTION.path,
        20,
        false,
        true,
        (270.0, 45.0),
        at(270.0, 45.0),
    );
    doc.text_at(
        layout::info::REQUIREMENTS.path,
        16,
        false,
        false,
        (500.0, 115.0),
        at(500.0, 115.0),
    );
    doc.text_at(
        layout::info::CONFIRM.path,
        22,
        false,
        false,
        (400.0, 110.0),
        at(400.0, 110.0),
    );

    let holder = (50.0, 645.0);
    doc.sprite_at(layout::xp::HOLDER.path, holder, holder);
    let back = doc.shape(
        [0.0, 0.0, 240.0, 46.0],
        &json!([{"type": "solid", "color": [40, 40, 40, 180]}]),
        &json!([]),
    );
    doc.put(
        layout::xp::BACKGROUND.path,
        back,
        "shape",
        matrix((1.0, 1.0), (0.0, 0.0)),
        matrix((1.0, 1.0), holder),
        &json!({}),
    );
    doc.text_at(
        layout::xp::TEXT.path,
        19,
        true,
        false,
        (7.0, 2.0),
        (57.0, 647.0),
    );
    let meter = doc.sprite(1);
    let meter_at = (holder.0 + 8.0, holder.1 + 26.0);
    doc.put(
        layout::xp::METER.path,
        meter,
        "sprite",
        matrix((1.0, 0.5), (8.0, 26.0)),
        matrix((1.0, 0.5), meter_at),
        &json!({}),
    );
    let outline = doc.stroked_shape(
        [-1.3, -1.3, 201.3, 26.3],
        [0.0, 0.0, 200.0, 25.0],
        &json!([]),
        &json!([{"width": 2, "color": [255, 255, 255, 255]}]),
    );
    doc.put(
        layout::xp::METER_OUTLINE.path,
        outline,
        "shape",
        matrix((1.0, 1.0), (0.0, 0.0)),
        matrix((1.0, 0.5), meter_at),
        &json!({}),
    );
    let fill = doc.sprite(1);
    doc.put(
        layout::xp::METER_FILL.path,
        fill,
        "sprite",
        matrix((2.0, 0.25), (0.0, 0.0)),
        matrix((2.0, 0.125), meter_at),
        &json!({}),
    );
    let fill_shape = doc.shape(
        [0.0, 0.0, 100.0, 100.0],
        &json!([{"type": "solid", "color": [255, 255, 255, 255]}]),
        &json!([]),
    );
    doc.put(
        layout::xp::METER_FILL_SHAPE.path,
        fill_shape,
        "shape",
        matrix((1.0, 1.0), (0.0, 0.0)),
        matrix((2.0, 0.125), meter_at),
        &json!({}),
    );
    doc.sprite_at(layout::hints::HINT_BAR.path, (111.0, 222.0), (111.0, 222.0));
}

fn numbers(doc: &mut Doc) {
    doc.number(&layout::grid::MIN_SCALE, 0.5);
    doc.number(&layout::grid::MAX_SCALE, 1.0);
    doc.number(&layout::grid::CLOSE_ENOUGH, 0.001);
    doc.number(&layout::grid::HOVER_X_MIN, 30.0);
    doc.number(&layout::grid::HOVER_X_MAX, 1200.0);
    doc.number(&layout::grid::HOVER_Y_MIN, 12.0);
    doc.number(&layout::grid::HOVER_Y_MAX, 700.0);
    doc.number(&layout::grid::HOVER_X_SPEED, 20.0);
    doc.number(&layout::grid::HOVER_Y_SPEED, 10.0);
    doc.number(&layout::grid::MAX_X, 0.0);
    doc.number(&layout::grid::MAX_Y, 0.0);
    doc.number(&layout::grid::TEXTURE_WIDTH, 2560.0);
    doc.number(&layout::grid::TOP_HEIGHT, 1500.0);
    doc.number(&layout::grid::BOTTOM_HEIGHT, 1500.0);
    doc.number(&layout::grid::BORDER_WIDTH, 1280.0);
    doc.number(&layout::grid::BORDER_HEIGHT, 720.0);
    doc.number(&layout::grid::MOUSE_DRAG_MOVE_SPEED, 1.0);
    doc.number(&layout::info::LOWER_Y, 600.0);
    doc.number(&layout::info::UPPER_Y, 30.0);
    doc.number(&layout::info::Y_BOUND, 450.0);
    doc.number(&filter::LUMINANCE_RED, 0.3086);
    doc.number(&filter::LUMINANCE_GREEN, 0.6094);
    doc.number(&filter::LUMINANCE_BLUE, 0.082);
}

fn literals(doc: &mut Doc) {
    let single = |doc: &mut Doc, key: &LiteralKey, value: f64| doc.literal(key, &json!([value]));
    doc.literal(&layout::cards::PLAY_FRAMES, &json!([2, 1]));
    doc.literal(&layout::cards::STOP_FRAMES, &json!([1, 2, 1]));
    single(doc, &layout::info::ENTRY_Y_OFFSET, 50.0);
    single(doc, &layout::stars::GRID_STAR_PITCH, 40.0);
    single(doc, &layout::stars::GRID_STAR_SCALE, 1.0);
    single(doc, &layout::stars::SPECIAL_STAR_PITCH, 20.0);
    single(doc, &layout::stars::SPECIAL_STAR_HALVES, 2.0);
    single(doc, &layout::stars::SPECIAL_STAR_CAP, 10.0);
    single(doc, &layout::info::STAR_GAP, 4.0);
    single(doc, &layout::info::REQUIREMENTS_GAP, 40.0);
    single(doc, &layout::info::CONFIRM_GAP, 30.0);
    single(doc, &layout::info::WIDTH_PADDING, 100.0);
    single(doc, &layout::info::WIDTH_HALVES, 2.0);
    single(doc, &layout::info::HEIGHT_PADDING, 30.0);
    single(doc, &layout::info::STAR_PITCH, 20.0);
    single(doc, &layout::info::STAR_SCALE, 0.5);
    single(doc, &layout::info::STAR_CENTRE, 2.0);
    single(doc, &layout::info::ERROR_TIMER_MS, 2000.0);
    single(doc, &layout::xp::BRACKET_LINE, 1.5);
    single(doc, &layout::xp::BRACKET_CORNER, 6.0);
    single(doc, &layout::xp::BRACKET_PADDING_X, 0.0);
    single(doc, &layout::xp::BRACKET_PADDING_Y, 0.0);
    single(doc, &layout::hints::BACKGROUND_COLOUR, 3_158_064.0);
    single(doc, &layout::hints::BACKGROUND_ALPHA, 0.5);
    doc.literal(&layout::xp::LEVEL_LABEL, &json!(["$$LEVEL "]));
    doc.literal(&layout::hints::CHOOSE_PREFIX, &json!(["$$Choose ("]));
    doc.literal(&layout::hints::CHOOSE_SUFFIX, &json!([")"]));
    doc.literal(
        &layout::hints::HINT_TEXTS,
        &json!(["$ACCEPT", "$CLOSE", "$PREV PERK", "$NEXT PERK"]),
    );
    doc.literal(
        &layout::hints::HINT_KEYS,
        &json!(["Enter", "Tab", "Ctrl", "Alt"]),
    );
    doc.literal(&layout::xp::BRACKET_STYLE, &json!(["horizontal"]));
    doc.literal(
        &layout::info::CONFIRM_PROMPT,
        &json!(["$ConfirmPerkSelect"]),
    );
    doc.literal(
        &layout::info::REQUIREMENTS_NOT_MET,
        &json!(["$PerksLowReqs"]),
    );
    single(doc, &filter::BRIGHTNESS, 50.0);
    single(doc, &filter::CONTRAST, -40.0);
    single(doc, &filter::SATURATION, -30.0);
    single(doc, &filter::HUE, 0.0);
    single(doc, &filter::CONTRAST_PERCENT, 100.0);
    single(doc, &filter::CONTRAST_NEGATIVE_RANGE, 127.0);
    single(doc, &filter::CONTRAST_NEGATIVE_MIDDLE, 127.0);
    single(doc, &filter::CONTRAST_MATRIX_RANGE, 127.0);
    single(doc, &filter::CONTRAST_MATRIX_HALF, 0.5);
    single(doc, &filter::CONTRAST_MATRIX_MIDDLE, 127.0);
    single(doc, &filter::SATURATION_PERCENT, 100.0);
    single(doc, &filter::SATURATION_NEGATIVE_BASE, 1.0);
    single(doc, &filter::SATURATION_MATRIX_ONE, 1.0);
}

/// The texts the fixture's translate keys stand for.
pub const TRANSLATIONS: [(&str, &str); 15] = [
    ("$STRENGTH", "Attribute A"),
    ("$PERCEPTION", "Attribute B"),
    ("$ENDURANCE", "Attribute C"),
    ("$CHARISMA", "Attribute D"),
    ("$INTELLIGENCE", "Attribute E"),
    ("$AGILITY", "Attribute F"),
    ("$LUCK", "Attribute G"),
    ("$ACCEPT", "Hint accept"),
    ("$CLOSE", "Hint close"),
    ("$PREV PERK", "Hint previous"),
    ("$NEXT PERK", "Hint next"),
    ("$Choose (#)", "Hint choose (#)"),
    ("$ConfirmPerkSelect", "Prompt confirm"),
    ("$PerksLowReqs", "Notice refused"),
    ("$LEVEL", "Level word"),
];

/// The document text of the fixture movie.
pub fn level_up_menu_movie() -> String {
    let mut doc = Doc::default();
    grid(&mut doc);
    doc.sprite_at(layout::cards::STRAY.path, (460.0, 1500.0), (460.0, 1500.0));
    headers(&mut doc);
    stars(&mut doc);
    panels(&mut doc);
    numbers(&mut doc);
    literals(&mut doc);
    let set_text = set_text_parts();
    for (section, target) in [
        ("constants", &mut doc.constants),
        ("literals", &mut doc.literals),
    ] {
        for (key, value) in set_text[section].as_object().into_iter().flatten() {
            target.insert(key.clone(), value.clone());
        }
    }
    crate::ui_layout::test_support::movie_json(
        layout::MOVIE,
        &json!({
            "symbols": doc.symbols,
            "characters": doc.characters,
            "instances": doc.instances,
            "constants": doc.constants,
            "literals": doc.literals,
        }),
    )
}
