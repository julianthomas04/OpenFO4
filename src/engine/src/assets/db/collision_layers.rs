//! COLL collision layers, reduced to the one question the player collision
//! needs answered: does a shape on this Havok layer ever touch the player?
//!
//! A layer never touches the player when it is a trigger or sensor layer, or
//! when its collides-with list is known and names neither character layer.
//! Anything undecidable stays solid, so a gap in the data can only cost
//! performance, never let the player walk through geometry.

use std::collections::HashMap;
use std::hash::{DefaultHasher, Hash, Hasher};

use anyhow::Context;
use rusqlite::Connection;

pub const CHARACTER_CONTROLLER_EDITOR_ID: &str = "L_CHARCONTROLLER";
pub const CHARACTER_BUMPER_EDITOR_ID: &str = "L_CHARBUMPER";

/// COLL FNAM flag bits that mark a layer as not a physical collider.
pub mod layer_flags {
    pub const TRIGGER_VOLUME: i64 = 0x1;
    pub const SENSOR: i64 = 0x2;
}

/// One `collision_layers` row.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CollisionLayerRow {
    pub form_key: String,
    pub editor_id: Option<String>,
    pub layer_index: Option<i64>,
    pub flags: Option<i64>,
    /// Form keys of the layers this one collides with; `None` = absent or unknown.
    pub collides_with: Option<Vec<String>>,
}

impl CollisionLayerRow {
    pub fn list_all(conn: &Connection) -> anyhow::Result<Vec<Self>> {
        let mut stmt = conn.prepare(
            "SELECT form_key, editor_id, layer_index, flags, collides_with FROM collision_layers",
        )?;
        let raw = stmt
            .query_map([], |row| {
                Ok((
                    row.get::<_, String>(0)?,
                    row.get::<_, Option<String>>(1)?,
                    row.get::<_, Option<i64>>(2)?,
                    row.get::<_, Option<i64>>(3)?,
                    row.get::<_, Option<String>>(4)?,
                ))
            })?
            .collect::<Result<Vec<_>, _>>()?;
        raw.into_iter()
            .map(|(form_key, editor_id, layer_index, flags, collides_with)| {
                let collides_with = collides_with
                    .map(|json| serde_json::from_str::<Vec<String>>(&json))
                    .transpose()
                    .with_context(|| {
                        format!("collision_layers {form_key}: collides_with is not a JSON array of form keys")
                    })?;
                Ok(Self {
                    form_key,
                    editor_id,
                    layer_index,
                    flags,
                    collides_with,
                })
            })
            .collect()
    }
}

#[derive(Debug, thiserror::Error, PartialEq, Eq)]
pub enum PlayerLayersError {
    #[error("collision layer {editor_id} is missing")]
    CharacterLayerMissing { editor_id: &'static str },
    #[error("collision layer index {index} of {form_key} is outside 0..=255")]
    LayerIndexOutOfRange { form_key: String, index: i64 },
    #[error("collision layer index {index} is carried by both {first} and {second}")]
    DuplicateLayerIndex {
        index: u8,
        first: String,
        second: String,
    },
}

/// What a shape's layer means for the player.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum LayerSolidity {
    /// Known to collide with the player, or not decidable as anything else.
    Solid,
    /// Known never to touch the player.
    NeverTouchesPlayer,
    /// Solid only because the layer's data is unknown (no row, or no collides-with list).
    SolidUnknown,
}

impl LayerSolidity {
    pub fn is_solid(self) -> bool {
        !matches!(self, Self::NeverTouchesPlayer)
    }
}

/// Per-layer solidity for the player, resolved once from the database.
#[derive(Debug, Clone, Default)]
pub struct PlayerCollisionLayers {
    /// `None` = nothing is known, every shape is solid.
    layers: Option<HashMap<u8, LayerSolidity>>,
}

impl PlayerCollisionLayers {
    /// Every shape stays solid (no filtering).
    pub fn all_solid() -> Self {
        Self { layers: None }
    }

    /// Resolves each row's solidity. Fails when either character layer is
    /// missing, since the rule cannot be evaluated without them.
    pub fn from_rows(rows: &[CollisionLayerRow]) -> Result<Self, PlayerLayersError> {
        let character_keys = [
            character_layer_key(rows, CHARACTER_CONTROLLER_EDITOR_ID)?,
            character_layer_key(rows, CHARACTER_BUMPER_EDITOR_ID)?,
        ];
        let mut layers: HashMap<u8, LayerSolidity> = HashMap::new();
        let mut owners: HashMap<u8, &str> = HashMap::new();
        for row in rows {
            let Some(index) = row.layer_index else {
                continue;
            };
            let layer =
                u8::try_from(index).map_err(|_| PlayerLayersError::LayerIndexOutOfRange {
                    form_key: row.form_key.clone(),
                    index,
                })?;
            if let Some(first) = owners.insert(layer, &row.form_key) {
                return Err(PlayerLayersError::DuplicateLayerIndex {
                    index: layer,
                    first: first.to_owned(),
                    second: row.form_key.clone(),
                });
            }
            layers.insert(layer, classify(row, &character_keys));
        }
        Ok(Self {
            layers: Some(layers),
        })
    }

    /// Equal exactly when both tables keep and drop the same layers; stable for one build of the
    /// engine, as `DefaultHasher` output may change between Rust releases.
    pub fn fingerprint(&self) -> u64 {
        let mut hasher = DefaultHasher::new();
        match &self.layers {
            None => hasher.write_u8(0),
            Some(layers) => {
                hasher.write_u8(1);
                let mut decisions: Vec<(u8, bool)> = layers
                    .iter()
                    .map(|(layer, solidity)| (*layer, solidity.is_solid()))
                    .collect();
                decisions.sort_unstable();
                decisions.hash(&mut hasher);
            }
        }
        hasher.finish()
    }

    /// A shape without a layer is undecodable and stays solid.
    pub fn solidity(&self, layer: Option<u8>) -> LayerSolidity {
        let (Some(layers), Some(layer)) = (&self.layers, layer) else {
            return LayerSolidity::Solid;
        };
        layers
            .get(&layer)
            .copied()
            .unwrap_or(LayerSolidity::SolidUnknown)
    }
}

fn character_layer_key<'a>(
    rows: &'a [CollisionLayerRow],
    editor_id: &'static str,
) -> Result<&'a str, PlayerLayersError> {
    rows.iter()
        .find(|row| row.editor_id.as_deref() == Some(editor_id))
        .map(|row| row.form_key.as_str())
        .ok_or(PlayerLayersError::CharacterLayerMissing { editor_id })
}

fn classify(row: &CollisionLayerRow, character_keys: &[&str; 2]) -> LayerSolidity {
    let flags = row.flags.unwrap_or(0);
    if flags & (layer_flags::TRIGGER_VOLUME | layer_flags::SENSOR) != 0 {
        return LayerSolidity::NeverTouchesPlayer;
    }
    match &row.collides_with {
        None => LayerSolidity::SolidUnknown,
        Some(list) => {
            let touches_player = list.iter().any(|key| {
                character_keys
                    .iter()
                    .any(|character| key.eq_ignore_ascii_case(character))
            });
            if touches_player {
                LayerSolidity::Solid
            } else {
                LayerSolidity::NeverTouchesPlayer
            }
        }
    }
}

/// Layer tables for tests elsewhere in the crate.
#[cfg(test)]
pub(crate) mod fixtures {
    use super::*;

    /// Character layers plus one trigger layer `layer`, which never touches the player.
    pub(crate) fn dropping_layer(layer: i64) -> PlayerCollisionLayers {
        let row = |form_key: &str, editor_id: &str, index: i64, flags: i64, list: &[&str]| {
            CollisionLayerRow {
                form_key: form_key.to_owned(),
                editor_id: Some(editor_id.to_owned()),
                layer_index: Some(index),
                flags: Some(flags),
                collides_with: Some(list.iter().map(|key| (*key).to_owned()).collect()),
            }
        };
        PlayerCollisionLayers::from_rows(&[
            row("A", CHARACTER_CONTROLLER_EDITOR_ID, 30, 0, &["B"]),
            row("B", CHARACTER_BUMPER_EDITOR_ID, 56, 0, &["A"]),
            row("C", "L_TRIGGER", layer, layer_flags::TRIGGER_VOLUME, &[]),
        ])
        .unwrap()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const CONTROLLER: &str = "08877F:Fallout4.esm";
    const BUMPER: &str = "0D8121:Fallout4.esm";

    fn layer(
        form_key: &str,
        editor_id: &str,
        index: i64,
        flags: i64,
        collides_with: Option<&[&str]>,
    ) -> CollisionLayerRow {
        CollisionLayerRow {
            form_key: form_key.to_owned(),
            editor_id: Some(editor_id.to_owned()),
            layer_index: Some(index),
            flags: Some(flags),
            collides_with: collides_with.map(|keys| keys.iter().map(|k| (*k).to_owned()).collect()),
        }
    }

    fn character_rows() -> Vec<CollisionLayerRow> {
        vec![
            layer(
                CONTROLLER,
                CHARACTER_CONTROLLER_EDITOR_ID,
                30,
                0,
                Some(&[BUMPER]),
            ),
            layer(
                BUMPER,
                CHARACTER_BUMPER_EDITOR_ID,
                56,
                0,
                Some(&[CONTROLLER]),
            ),
        ]
    }

    fn layers_with(extra: Vec<CollisionLayerRow>) -> PlayerCollisionLayers {
        let mut rows = character_rows();
        rows.extend(extra);
        PlayerCollisionLayers::from_rows(&rows).unwrap()
    }

    #[test]
    fn classify_handles_the_real_data_cases() {
        let keys = [CONTROLLER, BUMPER];
        let navcut = layer("N", "L_NAVCUT", 49, 5, None);
        assert_eq!(classify(&navcut, &keys), LayerSolidity::NeverTouchesPlayer);
        let noncollidable = layer("M", "L_NONCOLLIDABLE", 15, 0, Some(&[]));
        assert_eq!(
            classify(&noncollidable, &keys),
            LayerSolidity::NeverTouchesPlayer
        );
        let unknown = layer("U", "L_U", 20, 0, None);
        assert_eq!(classify(&unknown, &keys), LayerSolidity::SolidUnknown);
    }

    #[test]
    fn a_layer_listing_a_character_layer_stays_solid() {
        let layers = layers_with(vec![
            layer("000001:Fallout4.esm", "L_STATIC", 1, 0, Some(&[CONTROLLER])),
            layer(
                "000002:Fallout4.esm",
                "L_STAIRHELPER",
                31,
                0,
                Some(&[BUMPER]),
            ),
        ]);
        assert_eq!(layers.solidity(Some(1)), LayerSolidity::Solid);
        assert_eq!(layers.solidity(Some(31)), LayerSolidity::Solid);
    }

    #[test]
    fn a_layer_with_a_known_list_naming_no_character_layer_is_dropped() {
        let layers = layers_with(vec![layer(
            "000003:Fallout4.esm",
            "L_NONCOLLIDABLE",
            15,
            0,
            Some(&["000001:Fallout4.esm"]),
        )]);
        assert_eq!(layers.solidity(Some(15)), LayerSolidity::NeverTouchesPlayer);
    }

    #[test]
    fn an_empty_known_list_is_dropped() {
        let layers = layers_with(vec![layer(
            "000003:Fallout4.esm",
            "L_EMPTY",
            15,
            0,
            Some(&[]),
        )]);
        assert_eq!(layers.solidity(Some(15)), LayerSolidity::NeverTouchesPlayer);
    }

    #[test]
    fn trigger_and_sensor_flags_drop_a_layer_even_when_it_lists_the_player() {
        for flags in [0x1, 0x2, 0x3, 0x5] {
            let layers = layers_with(vec![layer(
                "000004:Fallout4.esm",
                "L_TRIGGER",
                12,
                flags,
                Some(&[CONTROLLER, BUMPER]),
            )]);
            assert_eq!(
                layers.solidity(Some(12)),
                LayerSolidity::NeverTouchesPlayer,
                "{flags}"
            );
        }
    }

    #[test]
    fn the_navmesh_obstacle_flag_alone_does_not_drop_a_layer() {
        let layers = layers_with(vec![layer("000005:Fallout4.esm", "L_X", 49, 0x4, None)]);
        assert_eq!(layers.solidity(Some(49)), LayerSolidity::SolidUnknown);
    }

    #[test]
    fn a_trigger_layer_without_a_list_is_dropped() {
        let layers = layers_with(vec![layer("000006:Fallout4.esm", "L_NAVCUT", 49, 5, None)]);
        assert_eq!(layers.solidity(Some(49)), LayerSolidity::NeverTouchesPlayer);
    }

    #[test]
    fn a_plain_layer_without_a_list_stays_solid_as_unknown() {
        let layers = layers_with(vec![layer("000007:Fallout4.esm", "L_Y", 20, 0, None)]);
        let solidity = layers.solidity(Some(20));
        assert_eq!(solidity, LayerSolidity::SolidUnknown);
        assert!(solidity.is_solid());
    }

    #[test]
    fn a_layer_number_without_a_row_stays_solid_as_unknown() {
        let layers = layers_with(vec![]);
        assert_eq!(layers.solidity(Some(99)), LayerSolidity::SolidUnknown);
    }

    #[test]
    fn a_shape_without_a_layer_stays_solid() {
        assert_eq!(layers_with(vec![]).solidity(None), LayerSolidity::Solid);
    }

    #[test]
    fn character_layers_are_found_by_editor_id_and_matched_by_form_key_ignoring_case() {
        let layers = layers_with(vec![layer(
            "000008:Fallout4.esm",
            "L_Z",
            21,
            0,
            Some(&["08877f:fallout4.esm"]),
        )]);
        assert_eq!(layers.solidity(Some(21)), LayerSolidity::Solid);
    }

    #[test]
    fn the_character_layers_themselves_are_solid_when_they_list_each_other() {
        let layers = layers_with(vec![]);
        assert_eq!(layers.solidity(Some(30)), LayerSolidity::Solid);
        assert_eq!(layers.solidity(Some(56)), LayerSolidity::Solid);
    }

    #[test]
    fn all_solid_never_drops_anything() {
        let layers = PlayerCollisionLayers::all_solid();
        assert_eq!(layers.solidity(Some(49)), LayerSolidity::Solid);
        assert_eq!(layers.solidity(None), LayerSolidity::Solid);
    }

    #[test]
    fn a_missing_character_layer_is_an_error() {
        for (kept, missing) in [
            (CHARACTER_CONTROLLER_EDITOR_ID, CHARACTER_BUMPER_EDITOR_ID),
            (CHARACTER_BUMPER_EDITOR_ID, CHARACTER_CONTROLLER_EDITOR_ID),
        ] {
            let rows: Vec<_> = character_rows()
                .into_iter()
                .filter(|row| row.editor_id.as_deref() == Some(kept))
                .collect();
            assert_eq!(
                PlayerCollisionLayers::from_rows(&rows).unwrap_err(),
                PlayerLayersError::CharacterLayerMissing { editor_id: missing }
            );
        }
    }

    #[test]
    fn rows_without_a_layer_index_are_ignored() {
        let mut orphan = layer("000009:Fallout4.esm", "L_NOINDEX", 0, 0, None);
        orphan.layer_index = None;
        assert!(
            PlayerCollisionLayers::from_rows(&[character_rows(), vec![orphan]].concat()).is_ok()
        );
    }

    #[test]
    fn an_out_of_range_or_duplicated_layer_index_is_an_error() {
        let mut rows = character_rows();
        rows.push(layer("00000A:Fallout4.esm", "L_BIG", 300, 0, None));
        assert!(matches!(
            PlayerCollisionLayers::from_rows(&rows).unwrap_err(),
            PlayerLayersError::LayerIndexOutOfRange { index: 300, .. }
        ));

        let mut rows = character_rows();
        rows.push(layer("00000B:Fallout4.esm", "L_DUP", 30, 0, None));
        assert!(matches!(
            PlayerCollisionLayers::from_rows(&rows).unwrap_err(),
            PlayerLayersError::DuplicateLayerIndex { index: 30, .. }
        ));
    }
}
