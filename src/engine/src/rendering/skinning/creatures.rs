use std::path::Path;

use tracing::{debug, warn};

use super::rig::{ANIM_INDEX_FILE, CHARACTER_ASSETS_DIR, CreatureDef};
use crate::assets::layout::MESHES_DIR;

/// The game's own folder of actor models below the meshes folder.
const ACTORS_DIR: &str = "Actors";
/// Folder levels below the actors root that are searched for animation indices
/// (`<Actor>/CharacterAssets/<Variant>` is three, `<Actor>/<Sub>/CharacterAssets` too).
const MAX_SCAN_DEPTH: usize = 4;

/// Units whose body is not a single `<Actor>.glb`; names are as `discover` forms them.
const BODY_OVERRIDES: &[(&str, &[&str])] = &[
    ("Alien", &["Alien_Body.glb"]),
    (
        "Character",
        &["MaleBody.glb", "MaleHands.glb", "BaseMaleHead.glb"],
    ),
    ("FeralGhoul", &["FeralGhoulBase.glb"]),
    ("Mosquito", &["BloodBugAdultBase.glb"]),
    ("Robot", &["MrHandy.glb"]),
    (
        "Supermutant",
        &["SupermutantBody.glb", "SupermutantHead.glb"],
    ),
    ("Vertibird", &["Vertbird.glb"]),
];

/// Creatures the debug spawner offers: every unit under `<asset_root>/meshes/Actors`
/// with a converter-written `skeleton.anims.json` and an unambiguous body that
/// exists on disk. A unit's skeleton, clip packs and bodies all sit beside its index.
pub fn discover(asset_root: &str) -> Vec<CreatureDef> {
    let mut found = Vec::new();
    let actors = Path::new(asset_root).join(MESHES_DIR).join(ACTORS_DIR);
    if !actors.is_dir() {
        warn!(
            "creatures: '{}' is not a directory; no creatures offered",
            actors.display()
        );
        return found;
    }
    scan(&actors, &mut Vec::new(), &mut found);
    found.sort_by(|a, b| a.name.cmp(&b.name));
    found
}

fn scan(dir: &Path, segments: &mut Vec<String>, found: &mut Vec<CreatureDef>) {
    let entries = match std::fs::read_dir(dir) {
        Ok(entries) => entries,
        Err(error) => {
            warn!("creatures: cannot read '{}': {error}", dir.display());
            return;
        }
    };
    let mut file_names = Vec::new();
    let mut subdirs = Vec::new();
    for entry in entries {
        let entry = match entry {
            Ok(entry) => entry,
            Err(error) => {
                warn!(
                    "creatures: cannot list an entry of '{}': {error}",
                    dir.display()
                );
                continue;
            }
        };
        let name = entry.file_name().to_string_lossy().into_owned();
        match entry.file_type() {
            Ok(kind) if kind.is_dir() => subdirs.push(name),
            Ok(_) => file_names.push(name),
            Err(error) => warn!(
                "creatures: cannot stat '{}': {error}",
                entry.path().display()
            ),
        }
    }
    if file_names.iter().any(|name| name == ANIM_INDEX_FILE)
        && let Some(def) = unit_def(dir, segments, &file_names)
    {
        found.push(def);
    }
    if segments.len() >= MAX_SCAN_DEPTH {
        return;
    }
    subdirs.sort();
    for subdir in subdirs {
        segments.push(subdir);
        scan(&dir.join(&segments[segments.len() - 1]), segments, found);
        segments.pop();
    }
}

fn unit_def(dir: &Path, segments: &[String], file_names: &[String]) -> Option<CreatureDef> {
    let parts: Vec<&str> = segments
        .iter()
        .map(String::as_str)
        .filter(|part| *part != CHARACTER_ASSETS_DIR)
        .collect();
    let name = parts.join("/");
    let wanted: Vec<String> = match BODY_OVERRIDES.iter().find(|(unit, _)| *unit == name) {
        Some((_, bodies)) => bodies.iter().map(|body| (*body).to_owned()).collect(),
        None => vec![format!("{}.glb", parts.last()?)],
    };
    let mut bodies = Vec::with_capacity(wanted.len());
    for body in &wanted {
        let Some(existing) = file_names
            .iter()
            .find(|candidate| candidate.eq_ignore_ascii_case(body))
        else {
            debug!(
                "creatures: '{name}' skipped, body '{body}' not found in '{}'",
                dir.display()
            );
            return None;
        };
        bodies.push(existing.clone());
    }
    Some(CreatureDef {
        name,
        assets_dir: format!("{MESHES_DIR}/{ACTORS_DIR}/{}", segments.join("/")),
        bodies,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn units_below_the_actors_folder_of_the_meshes_folder_are_offered_with_a_layout_rooted_path() {
        let root = tempfile::tempdir().unwrap();
        let unit = root
            .path()
            .join(MESHES_DIR)
            .join(ACTORS_DIR)
            .join("Dog")
            .join(CHARACTER_ASSETS_DIR);
        std::fs::create_dir_all(&unit).unwrap();
        std::fs::write(unit.join(ANIM_INDEX_FILE), "{}").unwrap();
        std::fs::write(unit.join("Dog.glb"), "").unwrap();

        let creatures = discover(root.path().to_str().unwrap());

        assert_eq!(creatures.len(), 1);
        assert_eq!(creatures[0].name, "Dog");
        assert_eq!(creatures[0].assets_dir, "meshes/Actors/Dog/CharacterAssets");
        assert_eq!(creatures[0].bodies, ["Dog.glb"]);
    }

    #[test]
    fn an_asset_root_without_the_meshes_folder_offers_no_creatures() {
        let root = tempfile::tempdir().unwrap();

        assert!(discover(root.path().to_str().unwrap()).is_empty());
    }
}
