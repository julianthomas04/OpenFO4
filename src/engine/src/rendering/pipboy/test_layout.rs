//! The Pip-Boy layout the tests run against: every declared key with neutral values, plus the
//! few values whose combination the modules check.

use std::sync::Arc;

use serde_json::json;

use super::pages::{
    LEVEL_UP_TRANSLATIONS, data_quests, data_stats, data_workshops, inv, level_up_menu_movie,
    radio, stat_perks, stat_special,
};
use super::scrolling_list::ListKeys;
use crate::ui_layout::UiLayout;
use crate::ui_layout::test_support::{FIXTURE_REGULAR_CLASS, instance, synthesized_documents};

/// The literals a list checks against each other: its entry class and its text option.
fn list_overlay(keys: &ListKeys) -> (&'static str, serde_json::Value) {
    (
        keys.movie,
        json!({"literals": {
            keys.entry_class.path: {"values": [keys.entry_symbol]},
            keys.text_option.path: {"values": [keys.shared.shrink_option.path]},
        }}),
    )
}

/// A two-frame flash: full opacity, then a quarter.
fn flashing() -> serde_json::Value {
    let key = |frame: u32, alpha: f32| {
        json!({"frame": frame, "matrix": [1, 0, 0, 1, 0, 0],
               "color": {"mult": [1, 1, 1, alpha], "add": [0, 0, 0, 0]}, "visible": true})
    };
    let mut flashing = instance(1, "sprite", 0.0, 0.0);
    flashing["frames"] = json!([1, 2]);
    flashing["track"] = json!([key(1, 1.0), key(2, 0.25)]);
    flashing
}

/// The flashing key of the hint bar over the 60 frames of its holder: full opacity, then a quarter.
fn holder_flashing() -> serde_json::Value {
    let mut flashing = flashing();
    flashing["frames"] = json!([1, HOLDER_FRAMES]);
    flashing["track"][1]["frame"] = json!(HOLDER_FRAMES / 2);
    flashing
}

/// The frames of the hint holder's timeline; the code's key names the last one.
const HOLDER_FRAMES: u32 = 60;

/// The hint holder's symbol: a default frame and the frame the flashing loop starts at.
fn hint_holder() -> serde_json::Value {
    let class = "ButtonBarMenu_fla.BSButtonHint_IconHolder_8";
    json!({
        "characters": {"91": {"kind": "sprite", "frameCount": HOLDER_FRAMES,
                              "labels": {"Default": 1, "Flashing": 2}}},
        "symbols": {class: 91},
        "literals": {
            format!("{class}.frame{HOLDER_FRAMES}#gotoAndPlay(0)"): {"values": ["Flashing"]},
        },
    })
}

/// The page titles side by side.
fn header_titles() -> serde_json::Value {
    let titles: serde_json::Map<String, serde_json::Value> =
        ["STAT", "INV", "DATA", "MAP", "RADIO"]
            .iter()
            .zip(0_u8..)
            .map(|(page, index)| {
                let x = TITLE_PITCH * f64::from(index);
                (
                    format!("Menu_mc/Header_mc/PageHeader_mc/{page}_tf"),
                    instance(3, "text", x, 0.0),
                )
            })
            .collect();
    json!({
        "instances": titles,
    })
}

/// How far apart the test layout puts the page titles; a title's field spans 100 from its x.
pub const TITLE_PITCH: f64 = 200.0;

/// The sub-tab strip's timeline: two labelled animations and the frame script that stops one.
fn header_strip() -> serde_json::Value {
    json!({
        "characters": {"90": {"kind": "sprite", "frameCount": 6, "labels": {"up": 2, "down": 4}}},
        "instances": {"Menu_mc/Header_mc/TabHeader_mc": instance(90, "sprite", 0.0, 0.0)},
        "literals": {
            "Pipboy_Header.redrawTabs#TabHeader_mc.gotoAndPlay(0)": {"values": ["up", "down"]},
            "PipboyMenu_fla.TabHeader_14.<init>#addFrameScript(0)": {"values": [0]},
            "PipboyMenu_fla.TabHeader_14.<init>#addFrameScript(2)": {"values": [4]},
        },
    })
}

pub fn pipboy_layout() -> Arc<UiLayout> {
    let bar = "Pipboy_BottomBar.redrawUIComponent#";
    let overlay = json!({
        "literals": {
            format!("{bar}((Info_mc.HP_tf.text+\"  \")+Math.max())+"): {"values": ["|"]},
            format!("{bar}((Info_mc.AP_tf.text+\"  \")+Math.floor())+"): {"values": ["|"]},
            format!("{bar}Math.floor()+"): {"values": ["|"]},
            format!("{bar}Info_mc.HP_tf.text+"): {"values": ["::"]},
            format!("{bar}Info_mc.AP_tf.text+"): {"values": ["::?|?", "::"]},
        },
        "constants": {"Shared.GlobalFunc.NUM_DAMAGE_TYPES": super::summary::TYPE_ORDER.len()},
    });
    let lists = [
        &stat_special::LIST,
        &stat_perks::LIST,
        &inv::LIST,
        &inv::COMPONENT_LIST,
        &inv::OWNER_LIST,
        &radio::LIST,
        &data_workshops::LIST,
        &data_stats::CATEGORIES,
        &data_stats::VALUES,
        &data_quests::QUESTS,
        &data_quests::OBJECTIVES,
    ]
    .map(list_overlay);
    let hint_bar = json!({
        "literals": {
            "Shared.AS3.BSButtonHint.GetExpectedFont#$1=": {"values": vec![FIXTURE_REGULAR_CLASS; 3]},
        },
        "instances": {
            "[Shared.AS3.BSButtonHint]/IconHolderInstance:Flashing/IconAnimInstance": holder_flashing(),
        },
    });
    let player_pulse = json!({"instances": {"[PlayerLocMarker]/@1": flashing()}});
    let mut overlays = vec![
        ("Pipboy_MapPage", player_pulse),
        ("PipboyMenu", overlay),
        ("PipboyMenu", header_titles()),
        ("PipboyMenu", header_strip()),
        ("ButtonBarMenu", hint_bar),
        ("ButtonBarMenu", hint_holder()),
    ];
    overlays.extend(lists);
    let mut documents = synthesized_documents(&[&super::LAYOUT_KEYS], &overlays);
    documents.movies.insert(
        super::pages::PERK_CHART_MOVIE.to_owned(),
        level_up_menu_movie(),
    );
    for (key, text) in LEVEL_UP_TRANSLATIONS {
        documents.translate["entries"][key] = json!(text);
    }
    Arc::new(documents.layout())
}
