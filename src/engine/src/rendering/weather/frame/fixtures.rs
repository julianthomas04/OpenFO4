//! Weathers and snapshots for the tests of the frame.

use serde_json::{Value, json};

use crate::rendering::lighting::exterior_weather::fixtures::slots_with_sky;
use crate::rendering::lighting::time_of_day::{SlotBlend, TimeSlot};
use crate::rendering::weather::clouds::Clouds;
use crate::rendering::weather::clouds::fixtures::clouds;
use crate::rendering::weather::definition::WeatherDef;
use crate::rendering::weather::definition::fixtures::weather_def;
use crate::rendering::weather::library::WeatherLibrary;
use crate::rendering::weather::library::fixtures::library_of;
use crate::sim::WeatherSnapshot;
use crate::weather::{FadeRange, SoundKind, WeatherId, WeatherSound};

pub(in crate::rendering::weather) const A: WeatherId = WeatherId::new(0);
pub(in crate::rendering::weather) const B: WeatherId = WeatherId::new(1);

pub(in crate::rendering::weather) const DAY: SlotBlend = SlotBlend {
    from: TimeSlot::Day,
    to: TimeSlot::Day,
    t: 0.0,
};
pub(in crate::rendering::weather) const NIGHT: SlotBlend = SlotBlend {
    from: TimeSlot::Night,
    to: TimeSlot::Night,
    t: 0.0,
};
pub(in crate::rendering::weather) const DUSK: SlotBlend = SlotBlend {
    from: TimeSlot::Day,
    to: TimeSlot::Night,
    t: 0.25,
};

/// The weathers `A` and `B`, in this order; both rainy, cloudy and without thunder.
pub(in crate::rendering::weather) fn library() -> WeatherLibrary {
    library_of(vec![weather("A"), weather("B")])
}

pub(in crate::rendering::weather) fn weather(form_key: &str) -> WeatherDef {
    weather_def(form_key)
}

pub(in crate::rendering::weather) fn settled(current: WeatherId) -> WeatherSnapshot<'static> {
    WeatherSnapshot {
        current,
        previous: None,
        progress: 1.0,
        strikes: &[],
        hours_to_next_roll: 8.0,
    }
}

/// `previous` turns into `current`, `progress` of the way.
pub(in crate::rendering::weather) fn transition(
    current: WeatherId,
    previous: WeatherId,
    progress: f32,
) -> WeatherSnapshot<'static> {
    WeatherSnapshot {
        current,
        previous: Some(previous),
        progress,
        strikes: &[],
        hours_to_next_roll: 8.0,
    }
}

/// The weather with an upper sky colour of its own for each time slot.
pub(in crate::rendering::weather) fn with_sky(
    weather: WeatherDef,
    sky_upper: impl Fn(TimeSlot) -> (u8, u8, u8),
) -> WeatherDef {
    WeatherDef {
        slots: slots_with_sky(&weather.form_key, "Sky", sky_upper),
        ..weather
    }
}

/// The weather with layers that carry `colour(slot)` and `alpha(slot)` in every layer.
pub(in crate::rendering::weather) fn with_clouds(
    weather: WeatherDef,
    colour: impl Fn(TimeSlot) -> [u8; 3],
    alpha: impl Fn(TimeSlot) -> f32,
) -> WeatherDef {
    let per_slot = |value: &dyn Fn(TimeSlot) -> Value| -> Value {
        TimeSlot::ALL
            .into_iter()
            .map(|slot| (slot.json_key().to_owned(), value(slot)))
            .collect()
    };
    let mut value = clouds();
    for layer in value["layers"].as_array_mut().unwrap() {
        layer["colors"] = per_slot(&|slot| {
            let [red, green, blue] = colour(slot);
            json!([red, green, blue, 0])
        });
        layer["alphas"] = per_slot(&|slot| json!(alpha(slot)));
    }
    WeatherDef {
        clouds: Some(Clouds::parse(&weather.form_key, &value.to_string()).unwrap()),
        ..weather
    }
}

pub(in crate::rendering::weather) fn without_clouds(weather: WeatherDef) -> WeatherDef {
    WeatherDef {
        clouds: None,
        ..weather
    }
}

pub(in crate::rendering::weather) fn dry(weather: WeatherDef) -> WeatherDef {
    WeatherDef {
        precipitation: None,
        ..weather
    }
}

pub(in crate::rendering::weather) fn with_precipitation_fade(
    weather: WeatherDef,
    begin_fade_in: f32,
    end_fade_out: f32,
) -> WeatherDef {
    WeatherDef {
        precipitation_fade: FadeRange {
            begin_fade_in,
            end_fade_out,
        },
        ..weather
    }
}

/// The weather with thunder sounding as its lightning fades in and out as given.
pub(in crate::rendering::weather) fn with_thunder(
    mut weather: WeatherDef,
    begin_fade_in: f32,
    end_fade_out: f32,
) -> WeatherDef {
    weather.sounds.push(WeatherSound {
        kind: SoundKind::Thunder,
        descriptor: "00BB02:Fallout4.esm".to_owned(),
    });
    weather.lightning.fade = FadeRange {
        begin_fade_in,
        end_fade_out,
    };
    weather
}
