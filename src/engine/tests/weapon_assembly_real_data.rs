//! Every placed weapon base of a converted installation, assembled from its default mods and the
//! converted GLBs: each mod with a model must find its connect point, the placed parts must form
//! one contiguous object, and the poses of the reference weapons must match the pinned values.
//!
//! Ignored by default (needs an installation converted by an installer that writes the weapon
//! columns and connect point extras). Run with
//! `cargo test --test weapon_assembly_real_data -- --ignored --nocapture`; the asset root follows
//! `OPENFO4_INSTALL_DIR` like the engine, the database is `OPENFO4_DB` when set, else
//! `<asset root>/data/data.db`.

use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::Arc;

use glam::{Mat4, Quat, Vec3};
use openfo4::assets::db::DbClient;
use openfo4::assets::db::weapons::WeaponRow;
use openfo4::assets::layout::data_dir;
use openfo4::assets::types::glb_key::glb_key;
use openfo4::assets::types::mesh::CpuMesh;
use openfo4::assets::types::weapon_assembly::{Assembly, ConnectInfo, ModRow, assemble};
use openfo4::assets::types::weapon_model::read_connect_info;
use openfo4::assets::{AssetRootEnv, resolve_asset_root};
use rusqlite::{Connection, OpenFlags};

/// Parts closer than this (engine units) count as touching.
const TOUCH_TOLERANCE: f32 = 0.01;
/// The vanilla missile launcher barrel leaves a 0.055 gap to its receiver.
const MISSILE_LAUNCHER: &str = "03F6F8:Fallout4.esm";
const MISSILE_LAUNCHER_TOUCH_TOLERANCE: f32 = 0.06;
const POSE_TOLERANCE: f32 = 1e-4;

fn touch_tolerance(form_key: &str) -> f32 {
    if form_key == MISSILE_LAUNCHER {
        MISSILE_LAUNCHER_TOUCH_TOLERANCE
    } else {
        TOUCH_TOLERANCE
    }
}

fn asset_root() -> PathBuf {
    resolve_asset_root(
        &AssetRootEnv::from_process(),
        Path::new(env!("CARGO_MANIFEST_DIR")),
    )
    .expect("set OPENFO4_INSTALL_DIR to a converted installation")
}

fn database_path(root: &Path) -> PathBuf {
    std::env::var_os("OPENFO4_DB").map_or_else(
        || PathBuf::from(format!("{}/data.db", data_dir(root.to_str().unwrap()))),
        PathBuf::from,
    )
}

fn placed_weapon_bases(path: &Path) -> Vec<String> {
    let conn = Connection::open_with_flags(path, OpenFlags::SQLITE_OPEN_READ_ONLY).unwrap();
    let mut stmt = conn
        .prepare(
            "SELECT DISTINCT base_object FROM placed_objects
             WHERE base_object IN (SELECT form_key FROM weapons)",
        )
        .unwrap();
    stmt.query_map([], |row| row.get(0))
        .unwrap()
        .map(Result::unwrap)
        .collect()
}

fn glb_path(root: &Path, model_path: &str) -> String {
    format!("{}/{}", root.to_str().unwrap(), glb_key(model_path))
}

fn connect_info(root: &Path, model_path: &str) -> Option<Arc<ConnectInfo>> {
    read_connect_info(root, model_path).ok().map(Arc::new)
}

type Bounds = (Vec3, Vec3);

fn part_bounds(root: &Path, model_path: &str, part_to_weapon: Mat4) -> Option<Bounds> {
    let meshes = CpuMesh::load(&glb_path(root, model_path)).ok()?;
    let mut bounds: Option<Bounds> = None;
    for vertex in meshes.iter().flat_map(|mesh| &mesh.vertices) {
        let point = part_to_weapon.transform_point3(Vec3::from(vertex.position));
        bounds = Some(bounds.map_or((point, point), |(min, max)| {
            (min.min(point), max.max(point))
        }));
    }
    bounds
}

fn gap(a: Bounds, b: Bounds) -> f32 {
    let separation = (a.0 - b.1).max(b.0 - a.1).max(Vec3::ZERO);
    separation.length()
}

/// Every bounded part can be reached from the first through parts that touch.
fn is_contiguous(bounds: &[Bounds], touch_tolerance: f32) -> bool {
    let Some(&first) = bounds.first() else {
        return true;
    };
    let mut reached = vec![first];
    let mut pending: Vec<Bounds> = bounds[1..].to_vec();
    loop {
        let before = pending.len();
        let mut apart = Vec::new();
        for candidate in pending {
            if reached
                .iter()
                .any(|known| gap(*known, candidate) <= touch_tolerance)
            {
                reached.push(candidate);
            } else {
                apart.push(candidate);
            }
        }
        pending = apart;
        if pending.is_empty() || pending.len() == before {
            return pending.is_empty();
        }
    }
}

struct Outcome {
    assembly: Assembly,
    expected_mod_parts: usize,
    contiguous: bool,
}

fn assemble_weapon(
    root: &Path,
    weapon: &WeaponRow,
    mods: &HashMap<String, ModRow>,
) -> Option<Outcome> {
    let base_model = weapon.model_path.as_deref()?;
    let default_mods = weapon.default_mods.as_deref().unwrap_or_default();
    let base_connect = connect_info(root, base_model)?;
    let assembly = assemble(
        base_model,
        base_connect,
        default_mods,
        |key| mods.get(key).cloned(),
        |model| connect_info(root, model),
    );
    let expected_mod_parts = default_mods
        .iter()
        .filter(|key| mods.get(*key).is_some_and(|row| row.model_path.is_some()))
        .count();
    let bounds: Vec<Bounds> = assembly
        .parts()
        .filter(|part| part.connect.has_geometry)
        .filter_map(|part| part_bounds(root, &part.model_path, part.part_to_weapon))
        .collect();
    Some(Outcome {
        contiguous: is_contiguous(&bounds, touch_tolerance(&weapon.form_key)),
        assembly,
        expected_mod_parts,
    })
}

struct Installation {
    root: PathBuf,
    placed: Vec<String>,
    weapons: Vec<WeaponRow>,
    mods: HashMap<String, ModRow>,
}

fn load_installation() -> Installation {
    let root = asset_root();
    let database = database_path(&root);
    let placed = placed_weapon_bases(&database);
    assert!(
        !placed.is_empty(),
        "no placed weapon bases in {}",
        database.display()
    );
    let db = DbClient::new(database.to_str().unwrap()).unwrap();
    let weapons = db
        .get_weapons()
        .unwrap()
        .expect("database predates weapon assembly: reinstall");
    let keys: Vec<String> = weapons
        .iter()
        .flat_map(|weapon| weapon.default_mods.iter().flatten().cloned())
        .collect();
    let mods = db
        .get_mod_models(&keys)
        .unwrap()
        .expect("database predates weapon assembly: reinstall")
        .into_iter()
        .map(|row| (row.form_key, row.row))
        .collect();
    Installation {
        root,
        placed,
        weapons,
        mods,
    }
}

#[test]
#[ignore = "needs a converted installation with weapon assembly data"]
fn every_placed_weapon_assembles_into_one_contiguous_object() {
    let Installation {
        root,
        placed,
        weapons,
        mods,
    } = load_installation();

    let mut failures = Vec::new();
    let mut parts_placed = 0;
    let mut parts_expected = 0;
    let placed_with_model = weapons
        .iter()
        .filter(|weapon| placed.contains(&weapon.form_key) && weapon.model_path.is_some());
    for weapon in placed_with_model {
        let label = format!(
            "{} ({})",
            weapon.form_key,
            weapon.editor_id.as_deref().unwrap_or("?")
        );
        let Some(outcome) = assemble_weapon(&root, weapon, &mods) else {
            failures.push(format!("{label}: base model missing or unreadable"));
            continue;
        };
        let mod_parts = outcome.assembly.mods.len();
        parts_placed += mod_parts;
        parts_expected += outcome.expected_mod_parts;
        if mod_parts != outcome.expected_mod_parts
            || !outcome.assembly.unmatched.is_empty()
            || !outcome.assembly.unresolved.is_empty()
        {
            failures.push(format!(
                "{label}: placed {mod_parts}/{} parts, unmatched {:?}, unresolved {:?}",
                outcome.expected_mod_parts, outcome.assembly.unmatched, outcome.assembly.unresolved
            ));
        }
        if !outcome.contiguous {
            failures.push(format!("{label}: parts are not contiguous"));
        }
    }
    println!(
        "{} placed weapon bases, {parts_placed}/{parts_expected} mod parts placed",
        placed.len()
    );
    assert!(failures.is_empty(), "{}", failures.join("\n"));
}

const NO_ROTATION: [f32; 4] = [0.0, 0.0, 0.0, 1.0];

struct ExpectedPart {
    attach_point: &'static str,
    translation: [f32; 3],
    rotation: [f32; 4],
}

struct ExpectedWeapon {
    form_key: &'static str,
    parts: Vec<ExpectedPart>,
}

const fn part(
    attach_point: &'static str,
    translation: [f32; 3],
    rotation: [f32; 4],
) -> ExpectedPart {
    ExpectedPart {
        attach_point,
        translation,
        rotation,
    }
}

/// Part poses of four reference weapons, one per kind of attachment: a gun with a rotated
/// receiver chain, a gun whose parts sit on child nodes, a melee grip whose point and root
/// rotations cancel, and a barrel with a half-turn root.
#[allow(
    clippy::unreadable_literal,
    clippy::excessive_precision,
    reason = "reference numbers are copied verbatim from the converter output"
)]
fn reference_weapons() -> Vec<ExpectedWeapon> {
    vec![
        ExpectedWeapon {
            form_key: "004822:Fallout4.esm",
            parts: vec![
                part(
                    "ap_gun_receiver",
                    [0.0; 3],
                    [0.0016746, 0.0010086, -0.0037573, 0.9999910],
                ),
                part(
                    "ap_gun_Grip",
                    [-0.003941, -0.237597, 0.059347],
                    [0.0033503, 0.0020183, -0.0075145, 0.9999641],
                ),
                part(
                    "ap_gun_Barrel",
                    [-0.003486, 0.493085, -1.849804],
                    [0.0016750, 0.0010090, -0.0037573, 0.9999910],
                ),
                part(
                    "ap_gun_Mag",
                    [-0.003489, -0.184850, 0.037262],
                    [0.0016749, 0.0010090, -0.0037573, 0.9999910],
                ),
            ],
        },
        ExpectedWeapon {
            form_key: "024F55:Fallout4.esm",
            parts: vec![
                part("ap_gun_receiver", [0.0; 3], NO_ROTATION),
                part(
                    "ap_gun_Barrel",
                    [0.001051, 0.101627, -2.638135],
                    NO_ROTATION,
                ),
                part("ap_gun_Grip", [0.0; 3], NO_ROTATION),
                part("ap_gun_Mag", [-0.249569, -0.065448, -0.962629], NO_ROTATION),
                part("ap_gun_Scope", [0.000465, 0.259985, -0.247886], NO_ROTATION),
            ],
        },
        ExpectedWeapon {
            form_key: "08E736:Fallout4.esm",
            parts: vec![part("ap_melee_Material", [0.0; 3], NO_ROTATION)],
        },
        ExpectedWeapon {
            form_key: MISSILE_LAUNCHER,
            parts: vec![part(
                "ap_gun_Barrel",
                [0.0, 1.098690, 1.027684],
                [0.0, -1.0, 0.0, 0.0],
            )],
        },
    ]
}

fn pose(translation: [f32; 3], rotation: [f32; 4]) -> Mat4 {
    Mat4::from_rotation_translation(
        Quat::from_xyzw(rotation[0], rotation[1], rotation[2], rotation[3]),
        Vec3::from(translation),
    )
}

fn assert_matrix_close(label: &str, actual: Mat4, expected: Mat4) {
    let worst = actual
        .to_cols_array()
        .iter()
        .zip(expected.to_cols_array())
        .map(|(a, e)| (a - e).abs())
        .fold(0.0, f32::max);
    assert!(
        worst <= POSE_TOLERANCE,
        "{label}: {actual:?} != {expected:?} (off by {worst})"
    );
}

#[test]
#[ignore = "needs a converted installation with weapon assembly data"]
fn the_reference_weapons_assemble_to_the_pinned_part_poses() {
    let installation = load_installation();
    let mut checked = 0;
    for expected in reference_weapons() {
        let weapon = installation
            .weapons
            .iter()
            .find(|weapon| weapon.form_key == expected.form_key)
            .unwrap_or_else(|| panic!("{} is not in the database", expected.form_key));
        let outcome = assemble_weapon(&installation.root, weapon, &installation.mods)
            .unwrap_or_else(|| panic!("{} did not assemble", expected.form_key));
        assert_eq!(
            outcome.assembly.mods.len(),
            expected.parts.len(),
            "{}: placed parts",
            expected.form_key
        );
        for expected_part in &expected.parts {
            let placed = outcome
                .assembly
                .mods
                .iter()
                .find(|placed| {
                    placed
                        .mod_key
                        .as_ref()
                        .and_then(|key| installation.mods.get(key))
                        .and_then(|row| row.attach_point_name.as_deref())
                        == Some(expected_part.attach_point)
                })
                .unwrap_or_else(|| {
                    panic!(
                        "{}: no part at {}",
                        expected.form_key, expected_part.attach_point
                    )
                });
            assert_matrix_close(
                &format!("{} {}", expected.form_key, expected_part.attach_point),
                placed.part_to_weapon,
                pose(expected_part.translation, expected_part.rotation),
            );
            checked += 1;
        }
    }
    println!("{checked} part poses match the pinned values");
}
