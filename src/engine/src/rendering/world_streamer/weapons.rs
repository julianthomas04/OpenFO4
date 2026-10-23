//! Placed weapons: a WEAP REFR draws and collides as its base model plus the
//! models of the default object-template mods, assembled once per base record
//! (`assets::types::weapon_assembly`) and shared by every REFR of that base.

use std::collections::{HashMap, HashSet};
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex, MutexGuard, OnceLock};

use glam::Mat4;

use super::{GroupInstances, GroupKey};

use crate::assets::db::weapons::{ModModelRow, WeaponRow};
use crate::assets::types::glb_key::glb_key;
use crate::assets::types::weapon_assembly::{self, ConnectInfo, ModRow};
use crate::assets::types::weapon_model::read_connect_info;
use crate::sim::collision::{CollisionMeta, PartLocal, Placement, manifest_key};
use crate::sim::refr_entity_id;
use crate::utils::warn_once;

const TRACE_ENV: &str = "OPENFO4_WEAPON_TRACE";

pub(super) fn trace_enabled() -> bool {
    static ENABLED: OnceLock<bool> = OnceLock::new();
    *ENABLED.get_or_init(|| std::env::var(TRACE_ENV).is_ok_and(|value| value == "1"))
}

/// A WEAP base record as the lookup stores it.
pub(super) struct WeaponBase {
    pub form_key: String,
    pub editor_id: String,
    pub default_mods: Vec<String>,
}

/// Every weapon base with a model, and the mod rows their default
/// combinations name. Bases without a model are reported and left out.
pub(super) struct WeaponRecords {
    pub bases: Vec<(WeaponBase, String)>,
    pub mods: HashMap<String, ModRow>,
}

impl WeaponRecords {
    pub fn load(
        rows: Vec<WeaponRow>,
        mod_rows: impl FnOnce(&[String]) -> Vec<ModModelRow>,
    ) -> Self {
        let mut bases = Vec::new();
        for row in rows {
            let editor_id = row.editor_id.unwrap_or_default();
            let Some(model_path) = row.model_path.filter(|path| !path.is_empty()) else {
                warn_once(&format!("weapon-no-model:{}", row.form_key), || {
                    format!(
                        "weapon {} ({editor_id}) has no model; its refs are not drawn",
                        row.form_key
                    )
                });
                continue;
            };
            bases.push((
                WeaponBase {
                    form_key: row.form_key,
                    editor_id,
                    default_mods: row.default_mods.unwrap_or_default(),
                },
                model_path,
            ));
        }
        let mut wanted: Vec<String> = bases
            .iter()
            .flat_map(|(base, _)| base.default_mods.iter().cloned())
            .collect();
        wanted.sort_unstable();
        wanted.dedup();
        let mods = mod_rows(&wanted)
            .into_iter()
            .map(|row| (row.form_key, row.row))
            .collect();
        Self { bases, mods }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum Skip {
    BaseUnreadable,
    NoCollider,
}

impl Skip {
    fn label(self) -> &'static str {
        match self {
            Self::BaseUnreadable => "base_unreadable",
            Self::NoCollider => "no_collider",
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub(super) struct PartCollision {
    pub dynamic: bool,
    pub mass: Option<f32>,
}

#[derive(Debug, Clone, PartialEq)]
pub(super) struct PlacedPart {
    /// Asset key of the part's GLB.
    pub mesh_path: String,
    pub part_to_weapon: Mat4,
    pub draw: bool,
    pub collision: Option<PartCollision>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum BodyKind {
    Dynamic,
    Static,
    None,
}

impl BodyKind {
    fn label(self) -> &'static str {
        match self {
            Self::Dynamic => "dynamic",
            Self::Static => "static",
            Self::None => "none",
        }
    }
}

/// The assembled parts of one weapon base; `parts` is empty when `skip` is set.
#[derive(Debug, Clone, PartialEq)]
pub(super) struct PlacedWeapon {
    pub form_key: String,
    pub editor_id: String,
    pub parts: Vec<PlacedPart>,
    pub skip: Option<Skip>,
    mod_parts: usize,
    unmatched: Vec<String>,
    unresolved: Vec<String>,
    duplicates: Vec<String>,
    no_root: usize,
}

impl PlacedWeapon {
    fn skipped(base: &WeaponBase, skip: Skip) -> Self {
        Self {
            form_key: base.form_key.clone(),
            editor_id: base.editor_id.clone(),
            parts: Vec::new(),
            skip: Some(skip),
            mod_parts: 0,
            unmatched: Vec::new(),
            unresolved: Vec::new(),
            duplicates: Vec::new(),
            no_root: 0,
        }
    }

    /// Nothing of a skipped weapon is placed; only the diagnostics stay.
    fn skip_all(self, skip: Skip) -> Self {
        Self {
            skip: Some(skip),
            parts: Vec::new(),
            mod_parts: 0,
            no_root: 0,
            ..self
        }
    }

    fn collisions(&self) -> impl Iterator<Item = PartCollision> + '_ {
        self.parts.iter().filter_map(|part| part.collision)
    }

    fn body_kind(&self) -> BodyKind {
        let mut collisions = self.collisions().peekable();
        if collisions.peek().is_none() {
            BodyKind::None
        } else if collisions.any(|collision| collision.dynamic) {
            BodyKind::Dynamic
        } else {
            BodyKind::Static
        }
    }

    /// The `OPENFO4_WEAPON_TRACE` line of one REFR of this weapon; a skipped
    /// weapon reports no parts and ends in `skipped=<reason>`.
    pub fn trace_line(&self, ref_key: &str) -> String {
        let meshes = self.parts.iter().filter(|part| part.draw).count();
        let mass = self
            .collisions()
            .filter_map(|collision| collision.mass)
            .fold(0.0_f32, |total, mass| total + mass);
        let skipped = self
            .skip
            .map_or_else(String::new, |skip| format!(" skipped={}", skip.label()));
        format!(
            "weapon_trace ref={ref_key} base={} edid={} parts={} meshes={meshes} colliders={} mass={mass:.3} body={} unmatched=[{}] unresolved=[{}] no_root={}{skipped}",
            self.form_key,
            self.editor_id,
            self.mod_parts,
            self.collisions().count(),
            self.body_kind().label(),
            self.unmatched.join(","),
            self.unresolved.join(","),
            self.no_root,
        )
    }
}

fn part_collision(
    manifest: &HashMap<String, CollisionMeta>,
    mesh_path: &str,
) -> Option<PartCollision> {
    manifest
        .get(&manifest_key(mesh_path))
        .map(|meta| PartCollision {
            dynamic: meta.dynamic,
            mass: meta.mass,
        })
}

/// Assembles one weapon base. `connect` reads the connect-point data of a
/// model path and is `None` for an unreadable model.
pub(super) fn place_weapon(
    base: &WeaponBase,
    base_model_path: &str,
    mods: &HashMap<String, ModRow>,
    manifest: &HashMap<String, CollisionMeta>,
    mut connect: impl FnMut(&str) -> Option<Arc<ConnectInfo>>,
) -> PlacedWeapon {
    let Some(base_connect) = connect(base_model_path) else {
        return PlacedWeapon::skipped(base, Skip::BaseUnreadable);
    };
    let assembly = weapon_assembly::assemble(
        base_model_path,
        base_connect,
        &base.default_mods,
        |key| mods.get(key).cloned(),
        connect,
    );
    let parts: Vec<PlacedPart> = assembly
        .parts()
        .map(|part| {
            let mesh_path = glb_key(&part.model_path);
            PlacedPart {
                collision: part_collision(manifest, &mesh_path),
                mesh_path,
                part_to_weapon: part.part_to_weapon,
                draw: part.connect.has_geometry,
            }
        })
        .collect();
    let placed = PlacedWeapon {
        form_key: base.form_key.clone(),
        editor_id: base.editor_id.clone(),
        mod_parts: assembly.mods.len(),
        unmatched: assembly.unmatched,
        unresolved: assembly.unresolved,
        duplicates: assembly.duplicates,
        no_root: assembly.no_root,
        skip: None,
        parts,
    };
    if placed.body_kind() == BodyKind::None {
        return placed.skip_all(Skip::NoCollider);
    }
    placed
}

/// Where a weapon REFR's parts are collected during cell assembly.
pub(super) struct WeaponSinks<'a> {
    pub placements_by_path: &'a mut HashMap<String, Vec<Placement>>,
    pub groups_by_key: &'a mut HashMap<GroupKey, GroupInstances>,
    pub expanded_refs: &'a mut HashSet<String>,
}

impl WeaponSinks<'_> {
    /// Whether this REFR is expanded for the first time in the batch; a
    /// repeated form key would double its colliders, mass and render rows.
    pub fn claim(&mut self, ref_key: Option<&str>) -> bool {
        let Some(ref_key) = ref_key else { return true };
        let first = self.expanded_refs.insert(ref_key.to_owned());
        if !first {
            warn_once(&format!("weapon-repeated-ref:{ref_key}"), || {
                format!(
                    "weapon REFR {ref_key} is placed more than once in one cell load; repeats ignored"
                )
            });
        }
        first
    }
}

/// The pose, uniform scale and form key of a placed weapon.
pub(super) struct WeaponRefr<'a> {
    pub world: Mat4,
    pub scale: f32,
    pub ref_key: Option<&'a str>,
}

pub(super) fn add_weapon_parts(placed: &PlacedWeapon, refr: &WeaponRefr, sinks: &mut WeaponSinks) {
    let (_, rotation, translation) = refr.world.to_scale_rotation_translation();
    let entity = refr.ref_key.map(refr_entity_id);
    for part in &placed.parts {
        if part.collision.is_some() {
            let (_, part_rotation, part_translation) =
                part.part_to_weapon.to_scale_rotation_translation();
            sinks
                .placements_by_path
                .entry(part.mesh_path.clone())
                .or_default()
                .push(Placement {
                    translation,
                    rotation,
                    scale: refr.scale,
                    ref_key: refr.ref_key.map(str::to_owned),
                    ragdoll_pose: None,
                    part_local: Some(PartLocal {
                        translation: part_translation,
                        rotation: part_rotation,
                    }),
                });
        }
        if part.draw {
            sinks
                .groups_by_key
                .entry((part.mesh_path.clone(), false, false, None, None, true))
                .or_default()
                .push(
                    refr.world * part.part_to_weapon,
                    entity,
                    Some(part.part_to_weapon),
                );
        }
    }
}

fn load_connect_info(asset_root: &Path, model_path: &str) -> Option<Arc<ConnectInfo>> {
    match read_connect_info(asset_root, model_path) {
        Ok(info) => {
            for rejected in &info.rejected {
                warn_once(
                    &format!(
                        "weapon-extras:{model_path}:{}:{}",
                        rejected.node_name, rejected.kind
                    ),
                    || {
                        format!(
                            "weapon model '{model_path}': {} extras on node '{}' ignored: {}",
                            rejected.kind, rejected.node_name, rejected.reason
                        )
                    },
                );
            }
            Some(Arc::new(info))
        }
        Err(error) => {
            warn_once(&format!("weapon-model:{model_path}"), || error.to_string());
            None
        }
    }
}

/// Locks `mutex`, recovering the data of a poisoned lock: the caches only
/// ever hold complete entries, so a panicked worker leaves them consistent.
fn lock_recovering<T>(mutex: &Mutex<T>) -> MutexGuard<'_, T> {
    mutex.lock().unwrap_or_else(|poisoned| {
        warn_once("weapon-cache-poisoned", || {
            "a worker panicked while holding the weapon assembly cache; continuing with its contents"
                .to_owned()
        });
        poisoned.into_inner()
    })
}

/// The weapon assemblies of a worker pool: built on first use, then shared.
/// Files are read and assemblies built outside the locks; when two workers
/// race on the same entry the first insert wins and both use it.
pub(super) struct WeaponAssemblies {
    asset_root: PathBuf,
    mods: HashMap<String, ModRow>,
    connect: Mutex<HashMap<String, Option<Arc<ConnectInfo>>>>,
    placed: Mutex<HashMap<String, Arc<PlacedWeapon>>>,
}

impl WeaponAssemblies {
    pub fn new(asset_root: &str, mods: HashMap<String, ModRow>) -> Self {
        Self {
            asset_root: PathBuf::from(asset_root),
            mods,
            connect: Mutex::default(),
            placed: Mutex::default(),
        }
    }

    fn connect_info(&self, model_path: &str) -> Option<Arc<ConnectInfo>> {
        if let Some(known) = lock_recovering(&self.connect).get(model_path) {
            return known.clone();
        }
        let loaded = load_connect_info(&self.asset_root, model_path);
        lock_recovering(&self.connect)
            .entry(model_path.to_owned())
            .or_insert(loaded)
            .clone()
    }

    pub fn placed(
        &self,
        base: &WeaponBase,
        base_model_path: &str,
        manifest: &HashMap<String, CollisionMeta>,
    ) -> Arc<PlacedWeapon> {
        if let Some(placed) = lock_recovering(&self.placed).get(&base.form_key) {
            return Arc::clone(placed);
        }
        let weapon = Arc::new(place_weapon(
            base,
            base_model_path,
            &self.mods,
            manifest,
            |path| self.connect_info(path),
        ));
        let mut placed = lock_recovering(&self.placed);
        if let Some(raced) = placed.get(&base.form_key) {
            return Arc::clone(raced);
        }
        report(&weapon);
        placed.insert(base.form_key.clone(), Arc::clone(&weapon));
        weapon
    }
}

fn report(weapon: &PlacedWeapon) {
    let (form_key, editor_id) = (&weapon.form_key, &weapon.editor_id);
    match weapon.skip {
        Some(Skip::BaseUnreadable) => warn_once(&format!("weapon-base:{form_key}"), || {
            format!(
                "weapon {form_key} ({editor_id}): base model unreadable; its refs are not drawn"
            )
        }),
        Some(Skip::NoCollider) => warn_once(&format!("weapon-no-collider:{form_key}"), || {
            format!(
                "weapon {form_key} ({editor_id}): no part has collision; its refs are not drawn"
            )
        }),
        None => {}
    }
    for attach_point in &weapon.unmatched {
        warn_once(
            &format!("weapon-unmatched:{form_key}:{attach_point}"),
            || {
                format!(
                    "weapon {form_key} ({editor_id}): no connect point for attach point {attach_point}; part skipped"
                )
            },
        );
    }
    for mod_key in &weapon.duplicates {
        warn_once(
            &format!("weapon-duplicate-mod:{form_key}:{mod_key}"),
            || {
                format!(
                    "weapon {form_key} ({editor_id}): mod {mod_key} is listed more than once in its default combination; placed once"
                )
            },
        );
    }
    for mod_key in &weapon.unresolved {
        warn_once(&format!("weapon-unresolved:{form_key}:{mod_key}"), || {
            format!(
                "weapon {form_key} ({editor_id}): mod {mod_key} has no row or unreadable model; part skipped"
            )
        });
    }
}

#[cfg(test)]
mod tests {
    use glam::{Quat, Vec3};

    use super::*;
    use crate::assets::types::weapon_assembly::ParentPoint;

    const BASE_MODEL: &str = "weapons\\gun\\dummy.nif";
    const GRIP_MODEL: &str = "weapons\\gun\\grip.nif";
    const GRIP_MESH: &str = "meshes\\weapons\\gun\\grip.glb";

    fn base() -> WeaponBase {
        WeaponBase {
            form_key: "004822:Fallout4.esm".to_owned(),
            editor_id: "10mm".to_owned(),
            default_mods: vec![
                "0231AB:Fallout4.esm".to_owned(),
                "04F21D:Fallout4.esm".to_owned(),
            ],
        }
    }

    fn mods() -> HashMap<String, ModRow> {
        HashMap::from([
            (
                "0231AB:Fallout4.esm".to_owned(),
                ModRow {
                    model_path: Some(GRIP_MODEL.to_owned()),
                    attach_point_name: Some("ap_gun_Grip".to_owned()),
                },
            ),
            (
                "04F21D:Fallout4.esm".to_owned(),
                ModRow {
                    model_path: None,
                    attach_point_name: Some("ap_gun_Muzzle".to_owned()),
                },
            ),
        ])
    }

    fn manifest(rows: &[(&str, bool, Option<f32>)]) -> HashMap<String, CollisionMeta> {
        rows.iter()
            .map(|(path, dynamic, mass)| {
                (
                    manifest_key(path),
                    CollisionMeta {
                        dynamic: *dynamic,
                        mass: *mass,
                        ragdoll: false,
                        water: None,
                    },
                )
            })
            .collect()
    }

    fn connect_info(path: &str) -> Arc<ConnectInfo> {
        let info = if path == BASE_MODEL {
            ConnectInfo {
                parents: vec![ParentPoint {
                    name: "P-Grip".to_owned(),
                    node_world: Mat4::IDENTITY,
                    local: Mat4::from_translation(Vec3::new(1.0, 2.0, 3.0)),
                }],
                child_root: None,
                has_geometry: false,
                rejected: vec![],
            }
        } else {
            ConnectInfo {
                parents: vec![],
                child_root: Some(Mat4::IDENTITY),
                has_geometry: true,
                rejected: vec![],
            }
        };
        Arc::new(info)
    }

    #[test]
    fn the_dummy_base_is_not_drawn_and_the_grip_sits_at_its_connect_point() {
        let manifest = manifest(&[
            ("meshes\\weapons\\gun\\dummy.glb", true, Some(0.5)),
            (GRIP_MESH, true, Some(1.0)),
        ]);
        let placed = place_weapon(&base(), BASE_MODEL, &mods(), &manifest, |path| {
            Some(connect_info(path))
        });
        assert_eq!(placed.skip, None);
        assert_eq!(placed.parts.len(), 2);
        assert!(!placed.parts[0].draw);
        assert_eq!(placed.parts[0].part_to_weapon, Mat4::IDENTITY);
        assert_eq!(placed.parts[1].mesh_path, GRIP_MESH);
        assert!(placed.parts[1].draw);
        assert_eq!(
            placed.parts[1].part_to_weapon.w_axis.truncate(),
            Vec3::new(1.0, 2.0, 3.0)
        );
    }

    #[test]
    fn the_trace_line_follows_the_contract_format() {
        let manifest = manifest(&[
            ("meshes\\weapons\\gun\\dummy.glb", true, Some(0.5)),
            (GRIP_MESH, true, Some(1.0)),
        ]);
        let placed = place_weapon(&base(), BASE_MODEL, &mods(), &manifest, |path| {
            Some(connect_info(path))
        });
        assert_eq!(
            placed.trace_line("00A1B2:Fallout4.esm"),
            "weapon_trace ref=00A1B2:Fallout4.esm base=004822:Fallout4.esm edid=10mm parts=1 meshes=1 colliders=2 mass=1.500 body=dynamic unmatched=[] unresolved=[] no_root=0"
        );
    }

    #[test]
    fn a_weapon_with_no_collider_in_any_part_is_skipped_entirely() {
        let placed = place_weapon(&base(), BASE_MODEL, &mods(), &manifest(&[]), |path| {
            Some(connect_info(path))
        });
        assert_eq!(placed.skip, Some(Skip::NoCollider));
        assert_eq!(placed.parts, []);
        assert_eq!(
            placed.trace_line("R"),
            "weapon_trace ref=R base=004822:Fallout4.esm edid=10mm parts=0 meshes=0 colliders=0 mass=0.000 body=none unmatched=[] unresolved=[] no_root=0 skipped=no_collider"
        );
    }

    #[test]
    fn a_skipped_weapon_keeps_its_diagnostics_in_the_trace() {
        let mut rows = mods();
        rows.insert(
            "0231AB:Fallout4.esm".to_owned(),
            ModRow {
                model_path: Some(GRIP_MODEL.to_owned()),
                attach_point_name: Some("ap_gun_Nowhere".to_owned()),
            },
        );
        let placed = place_weapon(&base(), BASE_MODEL, &rows, &manifest(&[]), |path| {
            Some(connect_info(path))
        });
        assert!(
            placed
                .trace_line("R")
                .contains("unmatched=[ap_gun_Nowhere] unresolved=[] no_root=0 skipped=no_collider")
        );
    }

    #[test]
    fn a_static_collider_alone_makes_a_static_body() {
        let manifest = manifest(&[(GRIP_MESH, false, None)]);
        let placed = place_weapon(&base(), BASE_MODEL, &mods(), &manifest, |path| {
            Some(connect_info(path))
        });
        assert!(
            placed
                .trace_line("R")
                .contains("colliders=1 mass=0.000 body=static")
        );
    }

    #[test]
    fn an_unreadable_base_model_skips_the_weapon() {
        let placed = place_weapon(&base(), BASE_MODEL, &mods(), &manifest(&[]), |_| None);
        assert_eq!(placed.skip, Some(Skip::BaseUnreadable));
    }

    #[test]
    fn unresolved_mods_and_unmatched_attach_points_are_listed_in_the_trace() {
        let mut rows = mods();
        rows.insert(
            "0231AB:Fallout4.esm".to_owned(),
            ModRow {
                model_path: Some(GRIP_MODEL.to_owned()),
                attach_point_name: Some("ap_gun_Nowhere".to_owned()),
            },
        );
        let mut weapon = base();
        weapon.default_mods.push("0AAAAA:Fallout4.esm".to_owned());
        let manifest = manifest(&[("meshes\\weapons\\gun\\dummy.glb", true, Some(0.5))]);
        let placed = place_weapon(&weapon, BASE_MODEL, &rows, &manifest, |path| {
            Some(connect_info(path))
        });
        let line = placed.trace_line("R");
        assert!(
            line.contains("parts=0 meshes=0 colliders=1 mass=0.500 body=dynamic unmatched=[ap_gun_Nowhere] unresolved=[0AAAAA:Fallout4.esm]"),
            "{line}"
        );
    }

    fn posed_weapon() -> PlacedWeapon {
        let part = |mesh_path: &str, part_to_weapon: Mat4, draw: bool| PlacedPart {
            mesh_path: mesh_path.to_owned(),
            part_to_weapon,
            draw,
            collision: Some(PartCollision {
                dynamic: true,
                mass: Some(1.0),
            }),
        };
        PlacedWeapon {
            parts: vec![
                part("meshes\\a.glb", Mat4::IDENTITY, false),
                part(
                    "meshes\\b.glb",
                    Mat4::from_rotation_translation(
                        Quat::from_rotation_x(0.7),
                        Vec3::new(1.0, 2.0, 3.0),
                    ),
                    true,
                ),
                part(
                    "meshes\\b.glb",
                    Mat4::from_translation(Vec3::new(-4.0, 0.0, 0.0)),
                    true,
                ),
            ],
            ..PlacedWeapon::skipped(&base(), Skip::NoCollider)
        }
    }

    fn refr_world_matrix() -> Mat4 {
        Mat4::from_scale_rotation_translation(
            Vec3::splat(2.0),
            Quat::from_rotation_y(1.1),
            Vec3::new(50.0, 5.0, -20.0),
        )
    }

    fn expand(
        weapon: &PlacedWeapon,
        ref_key: Option<&str>,
    ) -> (
        HashMap<String, Vec<Placement>>,
        HashMap<GroupKey, GroupInstances>,
    ) {
        let (mut placements, mut groups, mut expanded) =
            (HashMap::new(), HashMap::new(), HashSet::new());
        add_weapon_parts(
            weapon,
            &WeaponRefr {
                world: refr_world_matrix(),
                scale: 2.0,
                ref_key,
            },
            &mut WeaponSinks {
                placements_by_path: &mut placements,
                groups_by_key: &mut groups,
                expanded_refs: &mut expanded,
            },
        );
        (placements, groups)
    }

    fn assert_matrices_close(actual: Mat4, expected: Mat4) {
        let worst = actual
            .to_cols_array()
            .iter()
            .zip(expected.to_cols_array())
            .map(|(a, e)| (a - e).abs())
            .fold(0.0, f32::max);
        assert!(worst < 1e-4, "{actual:?} != {expected:?}");
    }

    #[test]
    fn a_part_row_is_the_refr_world_times_the_part_matrix_and_keeps_the_part_local() {
        let weapon = posed_weapon();
        let (_, groups) = expand(&weapon, Some("00A1B2:Fallout4.esm"));
        let key = ("meshes\\b.glb".to_owned(), false, false, None, None, true);
        let rows = &groups[&key];
        assert_eq!(rows.transforms.len(), 2);
        for (index, part) in weapon.parts[1..].iter().enumerate() {
            assert_matrices_close(
                rows.transforms[index],
                refr_world_matrix() * part.part_to_weapon,
            );
            assert_matrices_close(rows.locals[index], part.part_to_weapon);
        }
    }

    #[test]
    fn a_collision_placement_carries_the_refr_pose_and_the_part_matrix_decomposed() {
        let weapon = posed_weapon();
        let (placements, _) = expand(&weapon, Some("00A1B2:Fallout4.esm"));
        let placement = &placements["meshes\\b.glb"][0];
        let part = weapon.parts[1].part_to_weapon;
        let local = placement.part_local.unwrap();
        let recomposed = Mat4::from_scale_rotation_translation(
            Vec3::splat(placement.scale),
            placement.rotation,
            placement.translation,
        ) * Mat4::from_rotation_translation(local.rotation, local.translation);
        assert_matrices_close(recomposed, refr_world_matrix() * part);
        assert_eq!(placements["meshes\\b.glb"].len(), 2);
        assert_eq!(placement.ref_key.as_deref(), Some("00A1B2:Fallout4.esm"));
    }

    #[test]
    fn weapon_parts_group_apart_from_plain_placements_of_the_same_mesh() {
        let weapon = posed_weapon();
        let (mut placements, mut groups, mut expanded) =
            (HashMap::new(), HashMap::new(), HashSet::new());
        let plain_key = ("meshes\\b.glb".to_owned(), false, false, None, None, false);
        groups
            .entry(plain_key.clone())
            .or_insert_with(GroupInstances::default)
            .push(Mat4::IDENTITY, None, None);
        add_weapon_parts(
            &weapon,
            &WeaponRefr {
                world: refr_world_matrix(),
                scale: 2.0,
                ref_key: Some("R"),
            },
            &mut WeaponSinks {
                placements_by_path: &mut placements,
                groups_by_key: &mut groups,
                expanded_refs: &mut expanded,
            },
        );
        assert_eq!(groups.len(), 2);
        assert_eq!(groups[&plain_key].transforms.len(), 1);
    }

    #[test]
    fn a_repeated_weapon_refr_is_claimed_once_and_unkeyed_refs_always() {
        let (mut placements, mut groups, mut expanded) =
            (HashMap::new(), HashMap::new(), HashSet::new());
        let mut sinks = WeaponSinks {
            placements_by_path: &mut placements,
            groups_by_key: &mut groups,
            expanded_refs: &mut expanded,
        };
        assert!(sinks.claim(Some("R1")));
        assert!(!sinks.claim(Some("R1")));
        assert!(sinks.claim(Some("R2")));
        assert!(sinks.claim(None));
        assert!(sinks.claim(None));
    }

    #[test]
    fn one_mesh_at_two_offsets_within_a_weapon_keeps_both_rows() {
        let (placements, groups) = expand(&posed_weapon(), None);
        assert_eq!(placements["meshes\\b.glb"].len(), 2);
        let key = ("meshes\\b.glb".to_owned(), false, false, None, None, true);
        assert_eq!(groups[&key].transforms.len(), 2);
    }

    #[test]
    fn a_poisoned_cache_lock_is_recovered_with_its_contents() {
        let mutex = Arc::new(Mutex::new(vec![1, 2]));
        let poisoner = Arc::clone(&mutex);
        let _ = std::thread::spawn(move || {
            let _guard = poisoner.lock().unwrap();
            panic!("poison the lock");
        })
        .join();
        assert!(mutex.is_poisoned());
        assert_eq!(*lock_recovering(&mutex), [1, 2]);
    }

    #[test]
    fn records_without_a_model_are_left_out_and_default_mods_are_requested_once() {
        let rows = vec![
            WeaponRow {
                form_key: "A".to_owned(),
                editor_id: Some("a".to_owned()),
                model_path: Some("a.nif".to_owned()),
                default_mods: Some(vec!["M1".to_owned(), "M2".to_owned()]),
            },
            WeaponRow {
                form_key: "B".to_owned(),
                editor_id: None,
                model_path: Some("b.nif".to_owned()),
                default_mods: Some(vec!["M2".to_owned()]),
            },
            WeaponRow {
                form_key: "C".to_owned(),
                editor_id: Some("c".to_owned()),
                model_path: None,
                default_mods: None,
            },
        ];
        let mut requested = Vec::new();
        let records = WeaponRecords::load(rows, |keys| {
            requested = keys.to_vec();
            Vec::new()
        });
        assert_eq!(requested, ["M1", "M2"]);
        let keys: Vec<_> = records
            .bases
            .iter()
            .map(|(base, _)| base.form_key.as_str())
            .collect();
        assert_eq!(keys, ["A", "B"]);
    }
}
