use serde_json::json;

use super::keys::{
    FlagKey, FontClassKey, KeyCheck, LiteralKey, MainTimelineKey, NumberKey, PlacementKey,
    ShapeKey, StringKey, SymbolTimelineKey, TextKey, TimelineKey, TranslateKey,
};
use super::test_support::{
    FIXTURE_FONT_CLASS, assert_all_close, assert_close, font_config_json, instance,
    layout_with_movie, movie_json, text_character, translate_json,
};
use super::values::{Align, Edge};
use super::{UiLayout, UiLayoutError};

const MOVIE: &str = "Fixture";

fn fixture() -> UiLayout {
    let mut rotated = instance(10, "text", 0.0, 0.0);
    rotated["world"]["matrix"] = json!([0, 1, -1, 0, 0, 0]);
    let mut tracked = instance(10, "text", 5.0, 6.0);
    tracked["track"] = json!([
        {"frame": 1, "matrix": [1, 0, 0, 1, 5, 6], "color": {"mult": [1,1,1,1], "add": [0,0,0,0]}, "visible": true},
        {"frame": 9, "matrix": [1, 0, 0, 1, 50, 6], "color": {"mult": [1,1,1,1], "add": [0,0,0,0]}, "visible": false},
    ]);
    let mut html = text_character([-2.0, -2.0, 102.0, 40.0]);
    html["flags"]["html"] = json!(true);
    html["htmlFormat"] =
        json!({"TEXTFORMAT.LEADING": 4, "FONT.LETTERSPACING": -0.5, "P.ALIGN": "right"});
    let movie = movie_json(
        MOVIE,
        &json!({
            "frameCount": 3, "labels": {"Open": 2},
            "symbols": {"Entry": 12, "Mystery": 13},
            "characters": {
                "10": text_character([-2.0, -2.0, 192.0, 40.0]),
                "11": {"kind": "shape", "bounds": [0, 0, 100, 10],
                       "fills": [{"type": "solid", "color": [51, 51, 51, 255]}],
                       "lines": [{"width": 1.5, "color": [0, 0, 0, 255], "join": "miter",
                                  "miterLimit": 3, "startCap": "none", "endCap": "none"}],
                       "paths": [{"fillLeft": 0, "fillRight": 1, "line": 1, "start": [0, 0],
                                  "edges": [[100, 0], [100, 10], [60, 10, 0, 10], [0, 0]]}]},
                "12": {"kind": "sprite", "frameCount": 4, "labels": {"Selected": 3}},
                "13": {"kind": "bitmap", "width": 1, "height": 1},
                "14": html,
            },
            "instances": {
                "Bar/HP_tf": instance(10, "text", 10.0, 100.0),
                "Bar/Back": instance(11, "shape", 0.0, 0.0),
                "Bar/Clip": instance(12, "sprite", 0.0, 0.0),
                "Bar/Tracked": tracked,
                "Bar/Rotated": rotated,
                "Bar/Html": instance(14, "text", 0.0, 0.0),
                "Bar/Orphan": instance(99, "text", 0.0, 0.0),
                "Bar/Mismatch": instance(11, "text", 0.0, 0.0),
            },
            "constants": {"Shared.A.RATE": 0.65, "Shared.A.NAME": "x", "Shared.A.ON": true},
            "literals": {"Shared.A.run#go()": {"values": [1, 2]}},
            "unresolved": {"Shared.A.COMPUTED": "computed from an expression",
                           "Bar/Twice": "2 instances are named 'Twice' at frame 1"},
        }),
    );
    layout_with_movie(MOVIE, &movie)
}

#[test]
fn a_placement_gives_local_world_and_track() {
    let layout = fixture();
    let placement = layout
        .get(&PlacementKey::new(MOVIE, "Bar/Tracked"))
        .unwrap();
    assert_close(placement.world.matrix.tx, 5.0);
    assert_close(placement.local.matrix.ty, 6.0);
    assert_eq!(placement.track().len(), 2);
    assert!(!placement.track()[1].placement.visible);
}

#[test]
fn a_text_field_gives_character_data_and_placement() {
    let layout = fixture();
    let field = layout.get(&TextKey::new(MOVIE, "Bar/HP_tf")).unwrap();
    assert_close(field.size, 30.0);
    assert_eq!(field.font, FIXTURE_FONT_CLASS);
    assert_all_close(&field.bounds, &[-2.0, -2.0, 192.0, 40.0]);
    assert_close(field.leading, 2.0);
    assert_close(field.placement.world.matrix.tx, 10.0);
}

#[test]
fn html_text_formats_override_the_record_layout() {
    let layout = fixture();
    let field = layout.get(&TextKey::new(MOVIE, "Bar/Html")).unwrap();
    assert_close(field.leading, 4.0);
    assert_close(field.letter_spacing, -0.5);
    assert_eq!(field.align, Align::Right);
}

#[test]
fn a_shape_gives_bounds_fills_and_lines() {
    let layout = fixture();
    let shape = layout.get(&ShapeKey::new(MOVIE, "Bar/Back")).unwrap();
    assert_all_close(&shape.bounds, &[0.0, 0.0, 100.0, 10.0]);
    assert_eq!(shape.fills[0].color, Some([51, 51, 51, 255]));
    assert_close(shape.lines[0].width, 1.5);
    assert_eq!(shape.lines[0].join.as_deref(), Some("miter"));
    let path = &shape.paths[0];
    assert_eq!((path.fill_left, path.fill_right, path.line), (0, 1, 1));
    assert_all_close(&path.start, &[0.0, 0.0]);
    assert_eq!(path.edges[0], Edge::Straight([100.0, 0.0]));
    assert_eq!(path.edges[2], Edge::Curve([60.0, 10.0, 0.0, 10.0]));
    assert_all_close(&path.edges[2].end(), &[0.0, 10.0]);
}

#[test]
fn timelines_come_from_an_instance_a_symbol_or_the_movie() {
    let layout = fixture();
    let by_instance = layout.get(&TimelineKey::new(MOVIE, "Bar/Clip")).unwrap();
    let by_symbol = layout.get(&SymbolTimelineKey::new(MOVIE, "Entry")).unwrap();
    assert_eq!(by_instance, by_symbol);
    assert_eq!(by_symbol.frame_count, 4);
    assert_eq!(by_symbol.labels["Selected"], 3);
    let main = layout.get(&MainTimelineKey::of(MOVIE)).unwrap();
    assert_eq!((main.frame_count, main.labels["Open"]), (3, 2));
}

#[test]
fn constants_literals_translations_and_fonts_resolve() {
    let layout = fixture();
    assert_close(
        layout.get(&NumberKey::new(MOVIE, "Shared.A.RATE")).unwrap(),
        0.65,
    );
    assert_eq!(
        layout.get(&StringKey::new(MOVIE, "Shared.A.NAME")).unwrap(),
        "x"
    );
    assert!(layout.get(&FlagKey::new(MOVIE, "Shared.A.ON")).unwrap());
    let list = LiteralKey::new(MOVIE, "Shared.A.run#go()", 2);
    assert_close(layout.lit_number(&list, 1).unwrap(), 2.0);
    assert_eq!(
        layout.get(&TranslateKey::text("$FIXTURE_A")).unwrap(),
        "Fixture alpha"
    );
    assert_eq!(
        layout.get(&FontClassKey::class("$FIXTURE_Bold")).unwrap(),
        "Fixture Sans Bold"
    );
    assert_eq!(
        layout
            .get(&FontClassKey::class("$FIXTURE_Several"))
            .unwrap(),
        "Fixture Several"
    );
}

#[test]
fn a_font_class_takes_the_metrics_of_its_face_in_ems_and_a_face_without_metrics_is_an_error() {
    let mut fonts: serde_json::Value = serde_json::from_str(&font_config_json()).unwrap();
    fonts["fonts"] = json!([{"family": "Fixture Sans", "style": "Normal", "library": "f.swf",
                             "emSize": 2000, "ascent": 1500, "descent": 600, "leading": 0}]);
    let layout = UiLayout::from_documents(&translate_json(), &fonts.to_string(), &[]).unwrap();
    let metrics = layout.font_metrics(FIXTURE_FONT_CLASS).unwrap();
    assert_all_close(&[metrics.ascent, metrics.descent], &[0.75, 0.3]);
    let error = layout.font_metrics("$FIXTURE_Bold").unwrap_err();
    assert!(error.to_string().contains("Fixture Sans Bold"), "{error}");
}

#[test]
fn a_family_takes_the_metrics_of_its_face_without_a_font_class() {
    let mut fonts: serde_json::Value = serde_json::from_str(&font_config_json()).unwrap();
    fonts["fonts"] = json!([{"family": "Fixture Sans Bold", "style": "Bold", "library": "f.swf",
                             "emSize": 2000, "ascent": 1500, "descent": 600, "leading": 0}]);
    let layout = UiLayout::from_documents(&translate_json(), &fonts.to_string(), &[]).unwrap();
    let metrics = layout.family_metrics("Fixture Sans Bold").unwrap();
    assert_all_close(&[metrics.ascent, metrics.descent], &[0.75, 0.3]);
    let error = layout.family_metrics("Fixture Sans").unwrap_err();
    assert!(error.to_string().contains("Fixture Sans"), "{error}");
}

#[test]
fn a_family_with_several_styles_has_no_unambiguous_metrics() {
    let mut fonts: serde_json::Value = serde_json::from_str(&font_config_json()).unwrap();
    fonts["fonts"] = json!([
        {"family": "Fixture Sans", "style": "Normal", "library": "f.swf",
         "emSize": 1000, "ascent": 900, "descent": 300, "leading": 0},
        {"family": "Fixture Sans", "style": "Bold", "library": "f.swf",
         "emSize": 1000, "ascent": 900, "descent": 300, "leading": 0},
    ]);
    let layout = UiLayout::from_documents(&translate_json(), &fonts.to_string(), &[]).unwrap();
    let error = layout.family_metrics("Fixture Sans").unwrap_err();
    assert!(error.to_string().contains("several styles"), "{error}");
}

#[test]
fn a_movie_that_is_not_installed_is_a_missing_movie_error_naming_it() {
    let dir = tempfile::tempdir().unwrap();
    std::fs::write(dir.path().join("translate.json"), translate_json()).unwrap();
    std::fs::write(dir.path().join("fontconfig.json"), font_config_json()).unwrap();
    let layout = UiLayout::open(dir.path()).unwrap();
    let error = layout.get(&PlacementKey::new("Nowhere", "a")).unwrap_err();
    assert!(matches!(&error, UiLayoutError::MissingMovie { movie, .. } if movie == "Nowhere"));
    assert!(error.to_string().contains("Nowhere"));
}

#[test]
fn a_missing_key_names_movie_and_key_and_lists_siblings() {
    let layout = fixture();
    let error = layout
        .get(&PlacementKey::new(MOVIE, "Bar/AP_tf"))
        .unwrap_err();
    assert!(matches!(
        &error,
        UiLayoutError::MissingKey {
            kind: "instance",
            ..
        }
    ));
    let message = error.to_string();
    assert!(message.contains("Fixture") && message.contains("Bar/AP_tf"));
    assert!(message.contains("Bar/HP_tf"), "{message}");
}

#[test]
fn an_unresolved_key_reports_the_installers_reason() {
    let layout = fixture();
    let constant = layout
        .get(&NumberKey::new(MOVIE, "Shared.A.COMPUTED"))
        .unwrap_err();
    assert!(
        matches!(&constant, UiLayoutError::Unresolved { reason, .. } if reason == "computed from an expression")
    );
    let path = layout
        .get(&PlacementKey::new(MOVIE, "Bar/Twice"))
        .unwrap_err();
    assert!(matches!(path, UiLayoutError::Unresolved { .. }));
    let translation = layout.get(&TranslateKey::text("$DOUBLE")).unwrap_err();
    assert!(matches!(translation, UiLayoutError::Unresolved { .. }));
    let font = layout
        .get(&FontClassKey::class("$FIXTURE_Lost"))
        .unwrap_err();
    assert!(matches!(font, UiLayoutError::Unresolved { .. }));
}

#[test]
fn a_rotated_text_field_is_unresolved() {
    let layout = fixture();
    let error = layout.get(&TextKey::new(MOVIE, "Bar/Rotated")).unwrap_err();
    assert!(
        matches!(&error, UiLayoutError::Unresolved { reason, .. } if reason.contains("rotated"))
    );
}

#[test]
fn a_literal_list_of_the_wrong_length_is_a_count_mismatch() {
    let layout = fixture();
    let error = layout
        .lit_at(&LiteralKey::new(MOVIE, "Shared.A.run#go()", 1), 0)
        .unwrap_err();
    assert!(matches!(
        error,
        UiLayoutError::CountMismatch {
            expected: 1,
            actual: 2,
            ..
        }
    ));
}

#[test]
fn reading_a_key_as_the_wrong_kind_is_a_wrong_kind_error() {
    let layout = fixture();
    let shape_as_text = layout.get(&TextKey::new(MOVIE, "Bar/Back")).unwrap_err();
    assert!(
        matches!(&shape_as_text, UiLayoutError::WrongKind { expected: "a text field", actual, .. } if actual == "shape")
    );
    let number_as_string = layout
        .get(&StringKey::new(MOVIE, "Shared.A.RATE"))
        .unwrap_err();
    assert!(
        matches!(&number_as_string, UiLayoutError::WrongKind { actual, .. } if actual == "a number")
    );
    let text_as_sprite = layout
        .get(&TimelineKey::new(MOVIE, "Bar/HP_tf"))
        .unwrap_err();
    assert!(matches!(text_as_sprite, UiLayoutError::WrongKind { .. }));
}

fn movie_with(parts: &serde_json::Value) -> UiLayout {
    layout_with_movie(MOVIE, &movie_json(MOVIE, parts))
}

#[test]
fn a_literal_of_another_scalar_type_than_declared_fails_validation() {
    let layout = movie_with(&json!({"literals": {"A.f#x=": {"values": [1, "two"]}}}));
    let numbers = LiteralKey::new(MOVIE, "A.f#x=", 2);
    let texts = LiteralKey::texts(MOVIE, "A.f#x=", 2);
    let error = layout.validate(&[&numbers]).unwrap_err();
    let UiLayoutError::Invalid { failures } = &error else {
        panic!("expected one aggregated error, got {error}");
    };
    assert_eq!(failures.len(), 1);
    assert!(matches!(
        &failures[0],
        UiLayoutError::WrongKind { expected: "a number", actual, .. } if actual == "a string"
    ));
    let error = layout.validate(&[&texts]).unwrap_err();
    let UiLayoutError::Invalid { failures } = &error else {
        panic!("expected one aggregated error, got {error}");
    };
    assert_eq!(failures.len(), 1);
}

#[test]
fn whole_numbers_come_from_literals_and_constants_and_anything_else_is_unresolved() {
    let layout = movie_with(&json!({
        "literals": {"A.f#n": {"values": [7, 2.5, -1, 4_294_967_296.0]}},
        "constants": {"A.COUNT": 3, "A.HALF": 0.5},
    }));
    let literal = LiteralKey::new(MOVIE, "A.f#n", 4);
    assert_eq!(layout.lit_whole(&literal, 0).unwrap(), 7);
    for index in 1..4 {
        assert!(matches!(
            layout.lit_whole(&literal, index),
            Err(UiLayoutError::Unresolved { .. })
        ));
    }
    assert_eq!(
        layout
            .whole_constant(&NumberKey::new(MOVIE, "A.COUNT"))
            .unwrap(),
        3
    );
    assert!(matches!(
        layout.whole_constant(&NumberKey::new(MOVIE, "A.HALF")),
        Err(UiLayoutError::Unresolved { .. })
    ));
}

#[test]
fn an_html_format_that_is_not_the_expected_type_is_unresolved() {
    for (format, name) in [
        (json!({"TEXTFORMAT.LEADING": "wide"}), "leading"),
        (json!({"P.ALIGN": 3}), "align"),
    ] {
        let mut html = text_character([0.0, 0.0, 10.0, 10.0]);
        html["flags"]["html"] = json!(true);
        html["htmlFormat"] = format;
        let layout = movie_with(&json!({
            "characters": {"1": html},
            "instances": {"T": instance(1, "text", 0.0, 0.0)},
        }));
        let error = layout.get(&TextKey::new(MOVIE, "T")).unwrap_err();
        assert!(
            matches!(error, UiLayoutError::Unresolved { .. }),
            "{name}: {error}"
        );
    }
}

#[test]
fn a_shape_without_its_paths_is_a_bad_format_error() {
    let mut document: serde_json::Value = serde_json::from_str(&movie_json(
        MOVIE,
        &json!({"characters": {"1": {"kind": "shape", "bounds": [0, 0, 1, 1],
                                     "fills": [], "lines": [], "paths": []}}}),
    ))
    .unwrap();
    document["characters"]["1"]
        .as_object_mut()
        .unwrap()
        .remove("paths");
    let error = UiLayout::from_documents(
        &translate_json(),
        &font_config_json(),
        &[(MOVIE, &document.to_string())],
    )
    .err()
    .unwrap();
    assert!(matches!(error, UiLayoutError::BadFormat { .. }));
}

#[test]
fn a_format_other_than_one_is_a_hard_bad_format_error() {
    let mut document: serde_json::Value =
        serde_json::from_str(&movie_json(MOVIE, &json!({}))).unwrap();
    document["format"] = json!(2);
    let error = UiLayout::from_documents(
        &translate_json(),
        &font_config_json(),
        &[(MOVIE, &document.to_string())],
    )
    .err()
    .unwrap();
    assert!(
        matches!(&error, UiLayoutError::BadFormat { reason, .. } if reason.contains("format 2"))
    );
}

#[test]
fn a_document_of_the_wrong_kind_or_shape_is_a_bad_format_error() {
    let wrong_kind = translate_json();
    let error = UiLayout::from_documents(
        &translate_json(),
        &font_config_json(),
        &[(MOVIE, &wrong_kind)],
    )
    .err()
    .unwrap();
    assert!(matches!(error, UiLayoutError::BadFormat { .. }));
    let truncated = r#"{"format": 1, "kind": "movie"}"#;
    let error = UiLayout::from_documents(
        &translate_json(),
        &font_config_json(),
        &[(MOVIE, truncated)],
    )
    .err()
    .unwrap();
    assert!(error.to_string().contains(MOVIE));
}

#[test]
fn a_movie_file_is_read_lazily_and_cached() {
    let dir = tempfile::tempdir().unwrap();
    std::fs::write(dir.path().join("translate.json"), translate_json()).unwrap();
    std::fs::write(dir.path().join("fontconfig.json"), font_config_json()).unwrap();
    std::fs::write(dir.path().join("Lazy.json"), "{ not json").unwrap();
    let layout = UiLayout::open(dir.path()).unwrap();
    assert!(matches!(
        layout.movie("Lazy").err().unwrap(),
        UiLayoutError::BadFormat { .. }
    ));
    std::fs::write(dir.path().join("Lazy.json"), movie_json("Lazy", &json!({}))).unwrap();
    let first = layout.movie("Lazy").unwrap();
    std::fs::remove_file(dir.path().join("Lazy.json")).unwrap();
    assert!(std::sync::Arc::ptr_eq(
        &first,
        &layout.movie("Lazy").unwrap()
    ));
}

#[test]
fn validation_returns_one_error_listing_every_failure() {
    let layout = fixture();
    let fine = PlacementKey::new(MOVIE, "Bar/HP_tf");
    let missing = PlacementKey::new(MOVIE, "Bar/AP_tf");
    let wrong = TextKey::new(MOVIE, "Bar/Back");
    let lonely = LiteralKey::new(MOVIE, "Shared.A.run#go()", 5);
    let absent_movie = NumberKey::new("Gone", "X.Y");
    let checks: [&dyn KeyCheck; 5] = [&fine, &missing, &wrong, &lonely, &absent_movie];
    let error = layout.validate(&checks).unwrap_err();
    let UiLayoutError::Invalid { failures } = &error else {
        panic!("expected one aggregated error, got {error}");
    };
    assert_eq!(failures.len(), 4);
    let message = error.to_string();
    for needle in ["Bar/AP_tf", "Bar/Back", "Shared.A.run#go()", "Gone"] {
        assert!(message.contains(needle), "{needle} missing from {message}");
    }
    assert!(layout.validate(&[&fine]).is_ok());
}

/// Developer check against a real install: loads every document under `OPENFO4_INSTALL_DIR` and
/// resolves keys of the shipped movies.
#[test]
#[ignore = "needs OPENFO4_INSTALL_DIR with the installer's layout documents"]
fn the_installed_layout_documents_load_and_resolve_known_keys() {
    let install = std::env::var(crate::assets::INSTALL_DIR_VAR).expect("OPENFO4_INSTALL_DIR");
    let dir = std::path::PathBuf::from(crate::assets::layout::ui_layout_dir(&format!(
        "{install}/{}",
        crate::assets::layout::ASSETS_DIR
    )));
    let layout = UiLayout::open(&dir).unwrap();
    let mut names = Vec::new();
    collect_movies(&dir, &dir, &mut names);
    assert!(
        !names.is_empty(),
        "no movie documents under {}",
        dir.display()
    );
    for name in &names {
        layout.movie(name).unwrap_or_else(|error| panic!("{error}"));
    }
    let checks: [&dyn KeyCheck; 4] = [
        &crate::rendering::pipboy::LAYOUT_KEYS,
        &crate::ui_layout::set_text::SetTextKeys::new("PipboyMenu"),
        &NumberKey::new("Pipboy_InvPage", "Shared.GlobalFunc.DIMMED_ALPHA"),
        &TranslateKey::text("$STAT"),
    ];
    layout.validate(&checks).unwrap();
}

fn collect_movies(root: &std::path::Path, dir: &std::path::Path, names: &mut Vec<String>) {
    for entry in std::fs::read_dir(dir).unwrap() {
        let path = entry.unwrap().path();
        if path.is_dir() {
            collect_movies(root, &path, names);
        } else if path.extension().is_some_and(|ext| ext == "json") {
            let relative = path.strip_prefix(root).unwrap().with_extension("");
            let name = relative.to_string_lossy().replace('\\', "/");
            if name != "translate" && name != "fontconfig" {
                names.push(name);
            }
        }
    }
}
