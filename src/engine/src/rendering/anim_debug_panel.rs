use std::collections::HashMap;

use glam::{Mat4, Vec3};

use tracing::warn;

use crate::rendering::camera::Camera;
use crate::rendering::skinning::{
    CharacterSystem, SpawnRequest,
    character::{CharacterId, CharacterInstance, CharacterKind},
};
use crate::sim::SimWorld;

/// Horizontal distance in front of the camera, engine units (about 4 m).
pub const SPAWN_DISTANCE: f32 = 28.0;
/// How far above the eye the ground probe starts, so low ceilings and slopes
/// do not put the origin inside geometry (engine units).
const GROUND_PROBE_RISE: f32 = 2.0;
/// How far below the probe origin ground is searched for (engine units, about 14 m).
const GROUND_PROBE_RANGE: f32 = 100.0;
/// Drop from eye level to the ground used when no ground is found (engine units, about 1.8 m).
const FALLBACK_GROUND_DROP: f32 = 12.0;
const SPEED_RANGE: std::ops::RangeInclusive<f32> = 0.0..=3.0;
const CLIP_LIST_MAX_HEIGHT: f32 = 300.0;

/// Per character: which pack the clip list shows, how it is filtered and why
/// the last selection failed.
#[derive(Default)]
struct ClipPickerState {
    pack: usize,
    filter: String,
    error: Option<String>,
}

#[derive(Default)]
pub struct AnimDebugPanel {
    selected_creature: usize,
    pickers: HashMap<CharacterId, ClipPickerState>,
}

pub fn characters_section(
    ui: &mut egui::Ui,
    panel: &mut AnimDebugPanel,
    characters: &mut CharacterSystem,
    sim: &SimWorld,
    camera: &Camera,
) {
    egui::CollapsingHeader::new("Characters")
        .default_open(false)
        .show(ui, |ui| characters_body(ui, panel, characters, sim, camera));
}

fn characters_body(
    ui: &mut egui::Ui,
    panel: &mut AnimDebugPanel,
    characters: &mut CharacterSystem,
    sim: &SimWorld,
    camera: &Camera,
) {
    spawn_controls(ui, panel, characters, sim, camera);
    if let Some(error) = characters.spawn_error() {
        ui.colored_label(ui.visuals().error_fg_color, error);
    }

    ui.separator();
    panel.pickers.retain(|id, _| {
        characters
            .characters()
            .iter()
            .any(|character| character.id == *id)
    });
    let mut despawn = Vec::new();
    for character in characters
        .characters_mut()
        .iter_mut()
        .filter(|character| character.kind() == CharacterKind::Creature)
    {
        let picker = panel.pickers.entry(character.id).or_default();
        if character_controls(ui, character, picker) {
            despawn.push(character.id);
        }
    }
    for id in despawn {
        characters.queue_despawn(id);
    }
}

fn spawn_controls(
    ui: &mut egui::Ui,
    panel: &mut AnimDebugPanel,
    characters: &mut CharacterSystem,
    sim: &SimWorld,
    camera: &Camera,
) {
    if characters.creatures().is_empty() {
        ui.label("no creatures found; run the installer's animation conversion");
        return;
    }
    panel.selected_creature = panel
        .selected_creature
        .min(characters.creatures().len() - 1);
    egui::ComboBox::from_label("Creature")
        .selected_text(
            characters.creatures()[panel.selected_creature]
                .name
                .as_str(),
        )
        .show_ui(ui, |ui| {
            for (index, creature) in characters.creatures().iter().enumerate() {
                ui.selectable_value(&mut panel.selected_creature, index, creature.name.as_str());
            }
        });
    if ui.button("Spawn in front of camera").clicked() {
        let position = ground_position(sim, camera, SPAWN_DISTANCE, 0.0);
        characters.queue_spawn(SpawnRequest {
            creature: panel.selected_creature,
            transform: Mat4::from_translation(position),
        });
    }
}

/// The spot `distance` ahead of the camera (horizontally), `sideways` units to
/// its right, on the ground found by a downward ray. Without ground in range
/// it lies `FALLBACK_GROUND_DROP` below the eye.
pub fn ground_position(sim: &SimWorld, camera: &Camera, distance: f32, sideways: f32) -> Vec3 {
    let forward = camera.forward();
    let horizontal = Vec3::new(forward.x, 0.0, forward.z).normalize_or_zero();
    let right = horizontal.cross(Vec3::Y);
    let column = camera.eye + horizontal * distance + right * sideways;
    let probe = column + Vec3::Y * GROUND_PROBE_RISE;
    sim.ground_point_below(probe, GROUND_PROBE_RANGE + GROUND_PROBE_RISE).unwrap_or_else(|| {
        warn!("spawn: no ground within {GROUND_PROBE_RANGE} units below {probe:?}; using eye-relative height");
        column - Vec3::Y * FALLBACK_GROUND_DROP
    })
}

/// Returns whether the character should despawn.
fn character_controls(
    ui: &mut egui::Ui,
    character: &mut CharacterInstance,
    picker: &mut ClipPickerState,
) -> bool {
    let id = character.id;
    let title = format!("{} #{}", character.rig().name, id.0);
    let mut despawn = false;
    egui::CollapsingHeader::new(title)
        .id_salt(id)
        .default_open(true)
        .show(ui, |ui| {
            pack_picker(ui, id, character, picker);
            clip_picker(ui, id, character, picker);
            if let Some(error) = &picker.error {
                ui.colored_label(ui.visuals().error_fg_color, error);
            }
            if character.is_faulted() {
                ui.colored_label(
                    ui.visuals().error_fg_color,
                    "pose update failed; pick a clip to retry",
                );
            }
            let mut speed = character.speed();
            if ui
                .add(egui::Slider::new(&mut speed, SPEED_RANGE).text("speed"))
                .changed()
            {
                character.set_speed(speed);
            }
            match character.clip_duration() {
                Some(duration) => {
                    ui.label(format!("time: {:.2} / {:.2} s", character.time(), duration))
                }
                None => ui.label("rest pose"),
            };
            despawn = ui.button("Despawn").clicked();
        });
    despawn
}

fn pack_picker(
    ui: &mut egui::Ui,
    id: CharacterId,
    character: &CharacterInstance,
    picker: &mut ClipPickerState,
) {
    let packs = character.rig().packs();
    if packs.is_empty() {
        ui.label("no clip packs");
        return;
    }
    picker.pack = picker.pack.min(packs.len() - 1);
    let label =
        |index: usize| format!("{} ({} clips)", packs[index].file, packs[index].clips.len());
    egui::ComboBox::from_id_salt(("pack", id))
        .selected_text(label(picker.pack))
        .show_ui(ui, |ui| {
            egui::ScrollArea::vertical()
                .max_height(CLIP_LIST_MAX_HEIGHT)
                .show(ui, |ui| {
                    for index in 0..packs.len() {
                        ui.selectable_value(&mut picker.pack, index, label(index));
                    }
                });
        });
}

fn clip_picker(
    ui: &mut egui::Ui,
    id: CharacterId,
    character: &mut CharacterInstance,
    picker: &mut ClipPickerState,
) {
    let Some(pack) = character.rig().packs().get(picker.pack) else {
        return;
    };
    let playing_here = character
        .active_clip()
        .filter(|&(active_pack, _)| active_pack == picker.pack)
        .map(|(_, name)| name.to_owned());
    let mut picked = None;
    egui::ComboBox::from_id_salt(("clip", id))
        .selected_text(playing_here.as_deref().unwrap_or("select a clip"))
        .show_ui(ui, |ui| {
            ui.add(egui::TextEdit::singleline(&mut picker.filter).hint_text("filter"));
            let needle = picker.filter.to_lowercase();
            let matching: Vec<usize> = pack
                .clips
                .iter()
                .enumerate()
                .filter(|(_, name)| name.to_lowercase().contains(&needle))
                .map(|(index, _)| index)
                .collect();
            let row_height = ui.text_style_height(&egui::TextStyle::Button);
            egui::ScrollArea::vertical()
                .max_height(CLIP_LIST_MAX_HEIGHT)
                .show_rows(ui, row_height, matching.len(), |ui, rows| {
                    for &index in &matching[rows] {
                        let name = &pack.clips[index];
                        let selected = playing_here.as_deref() == Some(name.as_str());
                        if ui.selectable_label(selected, name).clicked() {
                            picked = Some(index);
                        }
                    }
                });
        });
    if let Some(clip) = picked {
        picker.error = character.set_clip(picker.pack, clip).err().map(|error| {
            tracing::error!("skin: {error}");
            error.to_string()
        });
    }
}
