//! Fixture documents written for the tests, in the layout format 1; no number or string of one is
//! copied from the game.

use serde_json::{Value, json};

use super::faces::{BOLD_FACE, REGULAR_FACE};
use super::layout::UiLayout;
use super::scaleform_text::FontMetrics;
use super::values::{Align, ColorTransform, LocalPlacement, Matrix, Placement, TextField, World};

pub fn assert_all_close(actual: &[f32], expected: &[f32]) {
    assert_eq!(actual.len(), expected.len());
    for (a, e) in actual.iter().zip(expected) {
        assert_close(*a, *e);
    }
}

pub fn assert_close(actual: impl Into<f64>, expected: impl Into<f64>) {
    let (actual, expected) = (actual.into(), expected.into());
    assert!(
        (actual - expected).abs() < 1e-4,
        "{actual} is not {expected}"
    );
}

pub const FIXTURE_FONT_CLASS: &str = "$FIXTURE_Font";
/// The metrics every fixture face declares, in font sizes.
pub const FIXTURE_METRICS: FontMetrics = FontMetrics {
    ascent: 0.8,
    descent: 0.2,
};
/// Font classes mapped to the faces the Pip-Boy loads.
pub const FIXTURE_REGULAR_CLASS: &str = "$FIXTURE_Regular";
pub const FIXTURE_BOLD_CLASS: &str = "$FIXTURE_Regular_Bold";

pub fn translate_json() -> String {
    json!({
        "format": 1, "extractor": 1, "kind": "translate", "language": "en",
        "entries": {"$FIXTURE_A": "Fixture alpha", "$FIXTURE_B": "Fixture beta"},
        "unresolved": {"$DOUBLE": "stated with 2 different texts"},
    })
    .to_string()
}

/// A face with [`FIXTURE_METRICS`] in a 1000-unit em.
pub fn fixture_face(family: &str, style: &str) -> Value {
    let em = 1000.0;
    json!({
        "family": family, "style": style, "library": "fixture.swf", "emSize": em,
        "ascent": FIXTURE_METRICS.ascent * em, "descent": FIXTURE_METRICS.descent * em,
        "leading": 0,
    })
}

pub fn font_config_json() -> String {
    json!({
        "format": 1, "extractor": 1, "kind": "fontconfig", "language": "en",
        "fontLibraries": [],
        "maps": [
            {"class": FIXTURE_REGULAR_CLASS, "family": REGULAR_FACE, "style": "Normal"},
            {"class": FIXTURE_BOLD_CLASS, "family": BOLD_FACE, "style": "Bold"},
            {"class": "$FIXTURE_Font", "family": "Fixture Sans", "style": "Normal"},
            {"class": "$FIXTURE_Bold", "family": "Fixture Sans Bold", "style": "Bold"},
            {"class": "$FIXTURE_Several", "family": "Fixture Several Bold", "style": "Bold"},
            {"class": "$FIXTURE_Several", "family": "Fixture Several", "style": "Normal"},
            {"class": "$FIXTURE_Lost", "family": "Fixture Lost A", "style": "Bold"},
            {"class": "$FIXTURE_Lost", "family": "Fixture Lost B", "style": "Italic"},
        ],
        "fonts": [
            fixture_face(REGULAR_FACE, "Normal"),
            fixture_face(BOLD_FACE, "Bold"),
            fixture_face("Fixture Sans", "Normal"),
            fixture_face("Fixture Sans Bold", "Bold"),
            fixture_face("Fixture Several Bold", "Bold"),
            fixture_face("Fixture Several", "Normal"),
            fixture_face("Fixture Lost A", "Bold"),
            fixture_face("Fixture Lost B", "Italic"),
        ],
        "validNameChars": null, "validBookChars": null,
    })
    .to_string()
}

pub fn identity_color() -> Value {
    json!({"mult": [1, 1, 1, 1], "add": [0, 0, 0, 0]})
}

pub fn instance(char: u32, kind: &str, tx: f64, ty: f64) -> Value {
    json!({
        "char": char, "kind": kind, "depth": 1, "name": "x", "frames": [1, 1],
        "local": {"matrix": [1, 0, 0, 1, tx, ty], "color": identity_color(), "visible": true},
        "world": {"matrix": [1, 0, 0, 1, tx, ty], "color": identity_color()},
    })
}

pub fn text_character(bounds: [f64; 4]) -> Value {
    json!({
        "kind": "text", "bounds": bounds, "font": FIXTURE_FONT_CLASS, "size": 30,
        "color": [255, 255, 255, 255], "align": "left", "leftMargin": 0, "rightMargin": 0,
        "indent": 0, "leading": 2,
        "flags": {"wordWrap": false, "multiline": false, "autoSize": false, "html": false},
    })
}

/// A complete movie document with the given `parts` overriding the empty sections.
pub fn movie_json(name: &str, parts: &Value) -> String {
    let mut document = json!({
        "format": 1, "extractor": 1, "kind": "movie", "movie": name,
        "stage": [0, 0, 1280, 720], "fps": 30, "frameCount": 1, "labels": {}, "symbols": {}, "characters": {}, "instances": {},
        "constants": {}, "literals": {}, "unresolved": {},
    });
    for (key, value) in parts.as_object().expect("parts are an object") {
        document[key] = value.clone();
    }
    document.to_string()
}

pub fn empty_movie() -> String {
    movie_json("Fixture", &json!({}))
}

/// The longest text the fixture's text-setting rule leaves untruncated.
pub const FIXTURE_TEXT_LIMIT: usize = 17;
/// How many characters of a truncated fixture text the ellipsis replaces.
pub const FIXTURE_ELLIPSIS_OVERHANG: usize = 2;
pub const FIXTURE_ELLIPSIS: &str = "~";

pub fn set_text_parts() -> Value {
    let rule = "Shared.GlobalFunc.SetText#";
    json!({
        "constants": {"Shared.GlobalFunc.MAX_TRUNCATED_TEXT_LENGTH": FIXTURE_TEXT_LIMIT},
        "literals": {
            format!("{rule}MAX_TRUNCATED_TEXT_LENGTH-"): {"values": [FIXTURE_ELLIPSIS_OVERHANG]},
            format!("{rule}$1.text.slice()+"): {"values": [FIXTURE_ELLIPSIS]},
            format!("{rule}$1.text.slice(0)"): {"values": [0]},
            format!("{rule}$2.charAt(0)"): {"values": [0]},
            format!("{rule}$2.charAt()=="): {"values": ["$"]},
            format!("{rule}$2="): {"values": [" "]},
            format!("{rule}$2=="): {"values": [""]},
        },
    })
}

/// A movie carrying the text-setting rule's constant and literals.
pub fn set_text_movie() -> String {
    movie_json("Fixture", &set_text_parts())
}

pub fn layout_with_movie(name: &str, movie: &str) -> UiLayout {
    UiLayout::from_documents(&translate_json(), &font_config_json(), &[(name, movie)])
        .expect("fixture documents are valid")
}

pub fn placement(tx: f32, ty: f32) -> Placement {
    let matrix = Matrix {
        a: 1.0,
        b: 0.0,
        c: 0.0,
        d: 1.0,
        tx,
        ty,
    };
    let color = ColorTransform {
        mult: [1.0; 4],
        add: [0.0; 4],
    };
    Placement {
        depth: 1,
        name: None,
        frames: [1, 1],
        local: LocalPlacement {
            matrix,
            color,
            visible: true,
            blend: None,
            clip_depth: None,
            ratio: None,
            cache_as_bitmap: None,
            class_name: None,
            filters: None,
        },
        world: World { matrix, color },
        track: None,
    }
}

pub fn text_field(bounds: [f32; 4], size: f32) -> TextField {
    TextField {
        bounds,
        font: FIXTURE_FONT_CLASS.to_owned(),
        size,
        color: None,
        align: Align::Left,
        left_margin: 0.0,
        right_margin: 0.0,
        indent: 0.0,
        leading: 0.0,
        letter_spacing: 0.0,
        multiline: false,
        word_wrap: false,
        auto_size: false,
        html: false,
        initial_text: None,
        placement: placement(0.0, 0.0),
    }
}

/// A layout that satisfies every key `checks` declares, with neutral values (identity
/// placements, 10 px filled and stroked shapes, 20 px text fields reading "text", constants and literals of 1, strings named after
/// their key, translations equal to their key): for tests of behaviour, not of placement.
/// `overlays` replace sections of a movie's synthesized document, e.g. a literal whose string
/// must equal another key's value.
pub fn synthesized_layout(checks: &[&dyn super::KeyCheck], overlays: &[(&str, Value)]) -> UiLayout {
    synthesized_documents(checks, overlays).layout()
}

/// The documents [`synthesized_layout`] loads.
pub fn synthesized_documents(
    checks: &[&dyn super::KeyCheck],
    overlays: &[(&str, Value)],
) -> LayoutDocuments {
    use super::keys::{DeclaredKind, ScalarKind};
    use std::collections::BTreeMap;

    let mut declared = Vec::new();
    for check in checks {
        check.declare(&mut declared);
    }
    let mut movies: BTreeMap<&str, Value> = BTreeMap::new();
    let mut translations = serde_json::Map::new();
    let mut font_classes: Vec<String> = Vec::new();
    for declaration in &declared {
        if let DeclaredKind::Translate = declaration.kind {
            translations.insert(declaration.path.to_owned(), json!(declaration.path));
            continue;
        }
        if let DeclaredKind::FontClass = declaration.kind {
            font_classes.push(declaration.path.to_owned());
            continue;
        }
        let movie = movies.entry(declaration.movie).or_insert_with(|| {
            json!({
                "characters": {
                    "1": {"kind": "sprite", "frameCount": 1, "labels": {}},
                    "2": {"kind": "shape", "bounds": [-1, -1, 11, 11], "edgeBounds": [0, 0, 10, 10],
                          "fills": [{"type": "solid", "color": [255, 255, 255, 255]}],
                          "lines": [{"width": 2, "color": [255, 255, 255, 255], "startCap": "none",
                                     "endCap": "none", "join": "miter", "miterLimit": 3}],
                          "paths": [{"fillLeft": 0, "fillRight": 1, "line": 1, "start": [0, 0],
                                     "edges": [[10, 0], [10, 10], [0, 10], [0, 0]]}]},
                    "3": {"kind": "text", "bounds": [-2, -2, 102, 24], "font": FIXTURE_REGULAR_CLASS,
                          "size": 20, "color": [255, 255, 255, 255], "align": "left",
                          "flags": {"wordWrap": false, "multiline": false, "autoSize": false,
                                    "html": false},
                          "initialText": "text"},
                },
                "instances": {}, "symbols": {}, "constants": {}, "literals": {},
            })
        });
        let path = declaration.path;
        let next_id = movie["characters"]
            .as_object()
            .map_or(0, serde_json::Map::len)
            + 1;
        match declaration.kind {
            DeclaredKind::Placement | DeclaredKind::Timeline => {
                movie["instances"][path] = instance(1, "sprite", 0.0, 0.0);
            }
            DeclaredKind::InstanceClass => {
                let id = next_id.to_string();
                movie["characters"][&id] =
                    json!({"kind": "sprite", "class": path, "frameCount": 1, "labels": {}});
                movie["instances"][path] = instance(next_id as u32, "sprite", 0.0, 0.0);
            }
            DeclaredKind::Text => movie["instances"][path] = instance(3, "text", 0.0, 0.0),
            DeclaredKind::Shape | DeclaredKind::Bounds => {
                movie["instances"][path] = instance(2, "shape", 0.0, 0.0);
            }
            DeclaredKind::SymbolTimeline => movie["symbols"][path] = json!(1),
            DeclaredKind::Label { label, symbol } => {
                let id = next_id.to_string();
                movie["characters"][&id] =
                    json!({"kind": "sprite", "frameCount": 2, "labels": {label: 2}});
                if symbol {
                    movie["symbols"][path] = json!(next_id);
                } else {
                    movie["instances"][path] = instance(next_id as u32, "sprite", 0.0, 0.0);
                }
            }
            DeclaredKind::Number => movie["constants"][path] = json!(1),
            DeclaredKind::Str => movie["constants"][path] = json!(path),
            DeclaredKind::Flag => movie["constants"][path] = json!(true),
            DeclaredKind::Literal { count, scalar } => {
                let value = match scalar {
                    ScalarKind::Number => json!(1),
                    ScalarKind::Text => json!(path),
                    ScalarKind::Flag => json!(true),
                };
                movie["literals"][path] = json!({"values": vec![value; count]});
            }
            DeclaredKind::MainTimeline
            | DeclaredKind::Stage
            | DeclaredKind::Translate
            | DeclaredKind::FontClass => {}
        }
    }
    for (name, overlay) in overlays {
        let movie = movies.entry(name).or_insert_with(|| json!({}));
        for (section, entries) in overlay.as_object().expect("overlays are objects") {
            for (key, value) in entries.as_object().expect("overlay sections are objects") {
                movie[section][key] = value.clone();
            }
        }
    }
    let mut translate: Value = serde_json::from_str(&translate_json()).unwrap();
    for (key, text) in translations {
        translate["entries"][key] = text;
    }
    let mut fonts: Value = serde_json::from_str(&font_config_json()).unwrap();
    for class in font_classes {
        let maps = fonts["maps"].as_array_mut().unwrap();
        maps.push(json!({"class": class, "family": REGULAR_FACE, "style": "Normal"}));
    }
    LayoutDocuments {
        translate,
        fonts,
        movies: movies
            .iter()
            .map(|(name, parts)| ((*name).to_owned(), movie_json(name, parts)))
            .collect(),
    }
}

/// The documents of a test layout, open to replacement before they are loaded.
pub struct LayoutDocuments {
    pub translate: Value,
    pub fonts: Value,
    pub movies: std::collections::BTreeMap<String, String>,
}

impl LayoutDocuments {
    pub fn layout(&self) -> UiLayout {
        let borrowed: Vec<(&str, &str)> = self
            .movies
            .iter()
            .map(|(name, json)| (name.as_str(), json.as_str()))
            .collect();
        UiLayout::from_documents(
            &self.translate.to_string(),
            &self.fonts.to_string(),
            &borrowed,
        )
        .expect("test documents are valid")
    }
}

/// The layout of the installation `OPENFO4_INSTALL_DIR` names, for the ignored real-install tests.
pub fn installed_layout() -> UiLayout {
    let install = std::env::var(crate::assets::INSTALL_DIR_VAR).expect("OPENFO4_INSTALL_DIR");
    let dir = std::path::PathBuf::from(crate::assets::layout::ui_layout_dir(&format!(
        "{install}/{}",
        crate::assets::layout::ASSETS_DIR
    )));
    UiLayout::open(&dir).expect("the installed layout opens")
}
