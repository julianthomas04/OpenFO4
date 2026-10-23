//! The Pip-Boy art the installer exports: `interface/pipboy/art_manifest.json` plus one SVG per
//! clip frame below `interface/pipboy/`. The manifest is the contract; see the fixed clip ids below.

use std::collections::{BTreeSet, HashMap, HashSet};
use std::path::{Component, Path, PathBuf};

use serde::Deserialize;

use crate::assets::layout::{INTERFACE_DIR, PIPBOY_ART_MANIFEST_FILE, PIPBOY_DIR};
use crate::assets::pipboy_playback::{self, PlaybackStart, ScriptAction, ScriptStep};
use crate::sim::pipboy::Special;

pub const MANIFEST_FORMAT: u32 = 3;

/// How a clip's SVG colours are meant: `Mask` art is white and drawn multiplied by the Pip-Boy
/// tint; `Full` art (perk chart cards) is drawn in its own colours.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum ArtColour {
    Mask,
    Full,
}

#[derive(Clone, Debug, PartialEq, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ArtClip {
    pub id: String,
    /// `Interface`-relative path of the movie the clip came from, as stored in the game archive
    /// (`Components\VaultBoys\Perks\StrongBack.swf`, `Pipboy_InvPage.swf`).
    pub source: String,
    /// For a perk-chart card: the `source` path of the perk Vault Boy movie it illustrates.
    #[serde(default)]
    pub illustrates: Option<String>,
    pub colour: ArtColour,
    pub fps: f32,
    /// What the clip's own frame scripts do to its playback; empty means it plays on and wraps.
    pub script: Vec<ScriptStep>,
    /// Symbol-local bounds `[x_min, y_min, x_max, y_max]` in stage pixels; every frame's SVG
    /// `viewBox` covers exactly this rectangle, so the registration point is at `(-x_min, -y_min)`
    /// inside the image.
    pub bounds: [f32; 4],
    /// Frame files relative to the manifest's folder, one per timeline frame (entries may repeat).
    pub frames: Vec<String>,
    /// For a part of a composite (a paper doll piece): where the clip's registration point sits
    /// inside the composite, in stage pixels. Absent for clips drawn on their own.
    #[serde(default)]
    pub offset: Option<[f32; 2]>,
    /// For a part of a composite: the red, green, blue and alpha multipliers (1 = unchanged) the
    /// composite applies to the clip.
    #[serde(default)]
    pub tint: Option<[f32; 4]>,
}

impl ArtClip {
    /// The 0-based frame the clip shows `elapsed` seconds after it started; see
    /// [`pipboy_playback::frame_at`].
    pub fn frame_at(&self, elapsed: f64, start: Option<PlaybackStart>) -> usize {
        pipboy_playback::frame_at(&self.script, self.frames.len(), self.fps, elapsed, start)
    }

    /// Whether the clip, started at frame 1, has shown its last frame by `elapsed` seconds.
    pub fn reaches_last_frame(&self, elapsed: f64) -> bool {
        pipboy_playback::reaches_frame(
            &self.script,
            self.frames.len(),
            self.fps,
            elapsed,
            None,
            self.frames.len(),
        )
    }
}

#[derive(Clone, Debug, PartialEq, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ArtManifest {
    pub format: u32,
    pub clips: Vec<ArtClip>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Icon {
    DamagePhysical,
    DamageEnergy,
    DamageRadiation,
    DamagePoison,
    DamageCryo,
    DamageFire,
    DamageDealtPhysical,
    DamageDealtEnergy,
    DamageDealtRadiation,
    DamageDealtPoison,
    DamageDealtCryo,
    DamageDealtFire,
    Ammo,
    Weapon,
    Armor,
    Weight,
    Caps,
    Favourite,
    Legendary,
    SearchTag,
    StarEmpty,
    StarFilled,
    StarAvailable,
    Population,
    Food,
    Water,
    Power,
    Defense,
    Beds,
    Happiness,
    Alert,
    RaiderSkullDiscovered,
    RaiderSkullUndiscovered,
    Timer,
    QuestCompleted,
    ComparisonBetter,
    ComparisonWorse,
    PlayerArrow,
    CustomMarker,
}

impl Icon {
    pub const ALL: [Icon; 39] = [
        Icon::DamagePhysical,
        Icon::DamageEnergy,
        Icon::DamageRadiation,
        Icon::DamagePoison,
        Icon::DamageCryo,
        Icon::DamageFire,
        Icon::DamageDealtPhysical,
        Icon::DamageDealtEnergy,
        Icon::DamageDealtRadiation,
        Icon::DamageDealtPoison,
        Icon::DamageDealtCryo,
        Icon::DamageDealtFire,
        Icon::Ammo,
        Icon::Weapon,
        Icon::Armor,
        Icon::Weight,
        Icon::Caps,
        Icon::Favourite,
        Icon::Legendary,
        Icon::SearchTag,
        Icon::StarEmpty,
        Icon::StarFilled,
        Icon::StarAvailable,
        Icon::Population,
        Icon::Food,
        Icon::Water,
        Icon::Power,
        Icon::Defense,
        Icon::Beds,
        Icon::Happiness,
        Icon::Alert,
        Icon::RaiderSkullDiscovered,
        Icon::RaiderSkullUndiscovered,
        Icon::Timer,
        Icon::QuestCompleted,
        Icon::ComparisonBetter,
        Icon::ComparisonWorse,
        Icon::PlayerArrow,
        Icon::CustomMarker,
    ];

    pub fn clip_id(self) -> &'static str {
        match self {
            Icon::DamagePhysical => "icon/damage_physical",
            Icon::DamageEnergy => "icon/damage_energy",
            Icon::DamageRadiation => "icon/damage_radiation",
            Icon::DamagePoison => "icon/damage_poison",
            Icon::DamageCryo => "icon/damage_cryo",
            Icon::DamageFire => "icon/damage_fire",
            Icon::DamageDealtPhysical => "icon/damage_dealt_physical",
            Icon::DamageDealtEnergy => "icon/damage_dealt_energy",
            Icon::DamageDealtRadiation => "icon/damage_dealt_radiation",
            Icon::DamageDealtPoison => "icon/damage_dealt_poison",
            Icon::DamageDealtCryo => "icon/damage_dealt_cryo",
            Icon::DamageDealtFire => "icon/damage_dealt_fire",
            Icon::Ammo => "icon/ammo",
            Icon::Weapon => "icon/weapon",
            Icon::Armor => "icon/armor",
            Icon::Weight => "icon/weight",
            Icon::Caps => "icon/caps",
            Icon::Favourite => "icon/favourite",
            Icon::Legendary => "icon/legendary",
            Icon::SearchTag => "icon/search_tag",
            Icon::StarEmpty => "icon/star_empty",
            Icon::StarFilled => "icon/star_filled",
            Icon::StarAvailable => "icon/star_available",
            Icon::Population => "icon/population",
            Icon::Food => "icon/food",
            Icon::Water => "icon/water",
            Icon::Power => "icon/power",
            Icon::Defense => "icon/defense",
            Icon::Beds => "icon/beds",
            Icon::Happiness => "icon/happiness",
            Icon::Alert => "icon/alert",
            Icon::RaiderSkullDiscovered => "icon/raider_skull_discovered",
            Icon::RaiderSkullUndiscovered => "icon/raider_skull_undiscovered",
            Icon::Timer => "icon/timer",
            Icon::QuestCompleted => "icon/quest_completed",
            Icon::ComparisonBetter => "icon/comparison_better",
            Icon::ComparisonWorse => "icon/comparison_worse",
            Icon::PlayerArrow => "icon/player_arrow",
            Icon::CustomMarker => "icon/custom_marker",
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum DollPiece {
    Body,
    Torso,
    LeftArm,
    RightArm,
    LeftLeg,
    RightLeg,
    Helmet,
    Goggles,
    GasMask,
    Clothes1,
    Clothes2,
}

impl DollPiece {
    pub const ALL: [DollPiece; 11] = [
        DollPiece::Body,
        DollPiece::Torso,
        DollPiece::LeftArm,
        DollPiece::RightArm,
        DollPiece::LeftLeg,
        DollPiece::RightLeg,
        DollPiece::Helmet,
        DollPiece::Goggles,
        DollPiece::GasMask,
        DollPiece::Clothes1,
        DollPiece::Clothes2,
    ];

    pub fn clip_id(self) -> &'static str {
        match self {
            DollPiece::Body => "doll/body",
            DollPiece::Torso => "doll/torso",
            DollPiece::LeftArm => "doll/left_arm",
            DollPiece::RightArm => "doll/right_arm",
            DollPiece::LeftLeg => "doll/left_leg",
            DollPiece::RightLeg => "doll/right_leg",
            DollPiece::Helmet => "doll/helmet",
            DollPiece::Goggles => "doll/goggles",
            DollPiece::GasMask => "doll/gas_mask",
            DollPiece::Clothes1 => "doll/clothes_1",
            DollPiece::Clothes2 => "doll/clothes_2",
        }
    }
}

/// Number of limb-condition body variants (`condition/body_00` .. `condition/body_16`).
pub const CONDITION_BODY_VARIANTS: u8 = 17;

/// Every clip the screen needs regardless of game data; a manifest missing one fails to load.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum FixedArt {
    Icon(Icon),
    Doll(DollPiece),
    /// Limb-condition body variant `0..CONDITION_BODY_VARIANTS`.
    ConditionBody(u8),
    ConditionHead,
    SpecialVaultBoy(Special),
    /// The favourites menu's key glyphs: one frame per favourite icon type, frame 0 empty.
    Hotkey,
    /// The quantity slider's arrow, pointing left; the right arrow is the same clip turned round.
    SliderArrow,
    /// The glyph at the centre of the favourites cross.
    FavouritesCentre,
    /// The idle Vault Boy of the quests list, an endless loop.
    QuestDefaultBoy,
    /// A list's scroll arrow, pointing up; the down arrow is the same clip turned round.
    ListScrollArrow,
    /// The Vault-Tec logo of the loading screen, an endless loop in the logo sprite's own
    /// coordinates; the loading screen's layout places it.
    LoadingIcon,
}

impl FixedArt {
    /// Every fixed clip a manifest must carry.
    pub fn all() -> impl Iterator<Item = FixedArt> {
        Icon::ALL
            .into_iter()
            .map(FixedArt::Icon)
            .chain(DollPiece::ALL.into_iter().map(FixedArt::Doll))
            .chain((0..CONDITION_BODY_VARIANTS).map(FixedArt::ConditionBody))
            .chain(std::iter::once(FixedArt::ConditionHead))
            .chain(Special::ALL.into_iter().map(FixedArt::SpecialVaultBoy))
            .chain([
                FixedArt::Hotkey,
                FixedArt::SliderArrow,
                FixedArt::FavouritesCentre,
                FixedArt::QuestDefaultBoy,
                FixedArt::ListScrollArrow,
                FixedArt::LoadingIcon,
            ])
    }

    pub fn clip_id(self) -> String {
        match self {
            FixedArt::Icon(icon) => icon.clip_id().to_owned(),
            FixedArt::Doll(piece) => piece.clip_id().to_owned(),
            FixedArt::ConditionBody(variant) => format!("condition/body_{variant:02}"),
            FixedArt::ConditionHead => "condition/head".to_owned(),
            FixedArt::SpecialVaultBoy(special) => format!("special/{}", special_name(special)),
            FixedArt::Hotkey => "icon/hotkey".to_owned(),
            FixedArt::SliderArrow => "inv/slider_arrow".to_owned(),
            FixedArt::FavouritesCentre => "inv/favourites_centre".to_owned(),
            FixedArt::QuestDefaultBoy => "quest/default_boy".to_owned(),
            FixedArt::ListScrollArrow => "list/scroll_arrow".to_owned(),
            FixedArt::LoadingIcon => "loading/vault_tec_logo".to_owned(),
        }
    }
}

fn special_name(special: Special) -> &'static str {
    match special {
        Special::Strength => "strength",
        Special::Perception => "perception",
        Special::Endurance => "endurance",
        Special::Charisma => "charisma",
        Special::Intelligence => "intelligence",
        Special::Agility => "agility",
        Special::Luck => "luck",
    }
}

/// The perk chart's poster textures, installed from the game's texture archives like any other
/// texture; the keys are asset-root-relative and matched ignoring case.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum PerkGridTexture {
    Top,
    Bottom,
    OverlayTop,
    OverlayBottom,
}

impl PerkGridTexture {
    pub const ALL: [PerkGridTexture; 4] = [
        PerkGridTexture::Top,
        PerkGridTexture::Bottom,
        PerkGridTexture::OverlayTop,
        PerkGridTexture::OverlayBottom,
    ];

    pub fn key(self) -> &'static str {
        match self {
            PerkGridTexture::Top => "textures/interface/perkgrid/perkgrid_top_d.dds",
            PerkGridTexture::Bottom => "textures/interface/perkgrid/perkgrid_bottom_d.dds",
            PerkGridTexture::OverlayTop => "textures/interface/perkgrid/perkgridoverlay_top_d.dds",
            PerkGridTexture::OverlayBottom => {
                "textures/interface/perkgrid/perkgridoverlay_bottom_d.dds"
            }
        }
    }
}

/// `marker/<type>` clip id of a map marker type.
pub fn marker_clip_id(marker_type: u16) -> String {
    format!("marker/{marker_type}")
}

#[derive(Debug, thiserror::Error)]
pub enum ArtManifestError {
    #[error("Pip-Boy art manifest '{path}' could not be read: {source}")]
    Read {
        path: PathBuf,
        source: std::io::Error,
    },
    #[error("Pip-Boy art manifest '{path}' is malformed: {source}")]
    Parse {
        path: PathBuf,
        source: serde_json::Error,
    },
    #[error("Pip-Boy art manifest format {found} is not the supported {MANIFEST_FORMAT}")]
    UnsupportedFormat { found: u32 },
    #[error("Pip-Boy art clip '{0}' appears twice")]
    DuplicateClip(String),
    #[error("Pip-Boy art clips '{first}' and '{second}' both come from the movie '{movie}'")]
    DuplicateSource {
        movie: String,
        first: String,
        second: String,
    },
    #[error("Pip-Boy art clips '{first}' and '{second}' both illustrate the movie '{illustrated}'")]
    DuplicateIllustration {
        illustrated: String,
        first: String,
        second: String,
    },
    #[error("Pip-Boy art clip '{clip}' is invalid: {reason}")]
    InvalidClip { clip: String, reason: &'static str },
    #[error("Pip-Boy art clip '{clip}' names frame '{path}', which does not exist")]
    MissingFrame { clip: String, path: PathBuf },
    #[error("Pip-Boy art manifest lacks the required clip '{0}'")]
    MissingFixedClip(String),
    #[error("Pip-Boy art manifest has no clip for the map marker types {0:?}")]
    MissingMarkerClips(Vec<u16>),
}

/// The loaded manifest, indexed for lookup by id, by source movie and by illustrated perk movie.
#[derive(Debug)]
pub struct ArtLibrary {
    root: PathBuf,
    clips: Vec<ArtClip>,
    by_id: HashMap<String, usize>,
    by_source: HashMap<String, usize>,
    by_illustrated: HashMap<String, usize>,
}

const VAULT_BOY_CLIP_PREFIX: &str = "vaultboy/";

impl ArtLibrary {
    /// Loads and validates `<asset_root>/interface/pipboy/art_manifest.json`: format, unique ids,
    /// sane clips, every frame file present, every [`FixedArt`] clip present.
    pub fn load(asset_root: &Path) -> Result<Self, ArtManifestError> {
        let root = asset_root.join(INTERFACE_DIR).join(PIPBOY_DIR);
        let manifest_path = root.join(PIPBOY_ART_MANIFEST_FILE);
        let text =
            std::fs::read_to_string(&manifest_path).map_err(|source| ArtManifestError::Read {
                path: manifest_path.clone(),
                source,
            })?;
        let manifest: ArtManifest =
            serde_json::from_str(&text).map_err(|source| ArtManifestError::Parse {
                path: manifest_path,
                source,
            })?;
        if manifest.format != MANIFEST_FORMAT {
            return Err(ArtManifestError::UnsupportedFormat {
                found: manifest.format,
            });
        }
        Self::index(root, manifest.clips)
    }

    fn index(root: PathBuf, clips: Vec<ArtClip>) -> Result<Self, ArtManifestError> {
        let mut by_id = HashMap::new();
        let mut by_source: HashMap<String, usize> = HashMap::new();
        let mut by_illustrated: HashMap<String, usize> = HashMap::new();
        let mut checked_frames = HashSet::new();
        for (index, clip) in clips.iter().enumerate() {
            validate_clip(clip)?;
            if by_id.insert(clip.id.clone(), index).is_some() {
                return Err(ArtManifestError::DuplicateClip(clip.id.clone()));
            }
            for frame in &clip.frames {
                if checked_frames.insert(frame.as_str()) && !root.join(frame).is_file() {
                    return Err(ArtManifestError::MissingFrame {
                        clip: clip.id.clone(),
                        path: root.join(frame),
                    });
                }
            }
            if clip.id.starts_with(VAULT_BOY_CLIP_PREFIX) {
                let source = normalize_swf_path(&clip.source);
                if let Some(&first) = by_source.get(&source) {
                    return Err(ArtManifestError::DuplicateSource {
                        movie: source,
                        first: clips[first].id.clone(),
                        second: clip.id.clone(),
                    });
                }
                by_source.insert(source, index);
            }
            if let Some(illustrated) = &clip.illustrates {
                let illustrated = normalize_swf_path(illustrated);
                if let Some(&first) = by_illustrated.get(&illustrated) {
                    return Err(ArtManifestError::DuplicateIllustration {
                        illustrated,
                        first: clips[first].id.clone(),
                        second: clip.id.clone(),
                    });
                }
                by_illustrated.insert(illustrated, index);
            }
        }
        if let Some(missing) = FixedArt::all()
            .map(FixedArt::clip_id)
            .find(|id| !by_id.contains_key(id))
        {
            return Err(ArtManifestError::MissingFixedClip(missing));
        }
        Ok(Self {
            root,
            clips,
            by_id,
            by_source,
            by_illustrated,
        })
    }

    pub fn clip(&self, id: &str) -> Option<&ArtClip> {
        self.by_id.get(id).map(|&index| &self.clips[index])
    }

    /// Fails when the manifest lacks the clip of any of `marker_types`, so a map marker the art
    /// does not cover stops the load instead of vanishing from the map.
    pub fn require_marker_clips(
        &self,
        marker_types: impl IntoIterator<Item = u16>,
    ) -> Result<(), ArtManifestError> {
        let missing: BTreeSet<u16> = marker_types
            .into_iter()
            .filter(|marker_type| !self.by_id.contains_key(&marker_clip_id(*marker_type)))
            .collect();
        if missing.is_empty() {
            Ok(())
        } else {
            Err(ArtManifestError::MissingMarkerClips(
                missing.into_iter().collect(),
            ))
        }
    }

    /// A required clip; present by construction.
    pub fn fixed(&self, art: FixedArt) -> &ArtClip {
        self.clip(&art.clip_id())
            .expect("`load` rejects a manifest without every fixed clip")
    }

    /// The Vault Boy clip (`vaultboy/...`) exported from `swf_path` (a perk or quest movie),
    /// matched after [`normalize_swf_path`].
    pub fn clip_for_swf(&self, swf_path: &str) -> Option<&ArtClip> {
        self.by_source
            .get(&normalize_swf_path(swf_path))
            .map(|&index| &self.clips[index])
    }

    /// The perk-chart card that illustrates the perk whose Vault Boy movie is `perk_swf`.
    pub fn perk_card_for_swf(&self, perk_swf: &str) -> Option<&ArtClip> {
        self.by_illustrated
            .get(&normalize_swf_path(perk_swf))
            .map(|&index| &self.clips[index])
    }

    /// The perk-chart card of the movie symbol `class` (`PerkClipd9784`), matched against the
    /// illustrated movie's file name with its underscores dropped.
    pub fn perk_card_for_class(&self, class: &str) -> Option<&ArtClip> {
        let wanted = class.to_lowercase();
        self.by_illustrated
            .iter()
            .find(|(path, _)| {
                let name = path.rsplit('/').next().unwrap_or(path);
                name.strip_suffix(".swf")
                    .is_some_and(|stem| stem.replace('_', "") == wanted)
            })
            .map(|(_, &index)| &self.clips[index])
    }

    /// Path of a frame's SVG; a frame index past the end clamps to the last frame.
    pub fn frame_path(&self, clip: &ArtClip, frame: usize) -> PathBuf {
        let last = clip.frames.len().saturating_sub(1);
        self.root.join(&clip.frames[frame.min(last)])
    }
}

fn validate_clip(clip: &ArtClip) -> Result<(), ArtManifestError> {
    let invalid = |reason| ArtManifestError::InvalidClip {
        clip: clip.id.clone(),
        reason,
    };
    let [x_min, y_min, x_max, y_max] = clip.bounds;
    if clip.id.is_empty() {
        return Err(invalid("the id is empty"));
    }
    if clip.frames.is_empty() {
        return Err(invalid("it has no frames"));
    }
    if !(clip.fps.is_finite() && clip.fps > 0.0) {
        return Err(invalid("fps must be positive"));
    }
    if !(clip.bounds.iter().all(|v| v.is_finite()) && x_max > x_min && y_max > y_min) {
        return Err(invalid("the bounds are degenerate"));
    }
    validate_script(clip).map_err(invalid)?;
    if clip.frames.iter().any(|frame| escapes_folder(frame)) {
        return Err(invalid("a frame path leaves the art folder"));
    }
    if clip
        .offset
        .is_some_and(|offset| !offset.iter().all(|v| v.is_finite()))
    {
        return Err(invalid("the offset is not finite"));
    }
    if clip
        .tint
        .is_some_and(|tint| !tint.iter().all(|v| v.is_finite() && *v >= 0.0))
    {
        return Err(invalid("the tint must be finite and not negative"));
    }
    Ok(())
}

fn validate_script(clip: &ArtClip) -> Result<(), &'static str> {
    let frames = u32::try_from(clip.frames.len()).unwrap_or(u32::MAX);
    let on_clip = |frame: u32| (1..=frames).contains(&frame);
    for step in &clip.script {
        if step.action == ScriptAction::Unknown {
            return Err("a script step has an unknown action");
        }
        if step.action.needs_target() != step.target.is_some() {
            return Err("a script step needs a target exactly when it jumps");
        }
        if !on_clip(step.frame) || step.target.is_some_and(|target| !on_clip(target)) {
            return Err("a script step names a frame outside the clip");
        }
    }
    Ok(())
}

fn escapes_folder(relative: &str) -> bool {
    let path = Path::new(relative);
    path.is_absolute()
        || path
            .components()
            .any(|c| !matches!(c, Component::Normal(_)))
}

/// Lowercase, forward slashes, no leading `interface/`: the key both a database SWF path and a
/// manifest `source` reduce to.
pub fn normalize_swf_path(path: &str) -> String {
    let lowered = path.trim().replace('\\', "/").to_lowercase();
    let trimmed = lowered.trim_start_matches('/');
    trimmed
        .strip_prefix("interface/")
        .unwrap_or(trimmed)
        .to_owned()
}

#[cfg(test)]
mod tests {
    use serde_json::{Value, json};

    use super::*;
    use crate::installer_source;

    #[test]
    fn the_manifest_format_is_the_one_the_installer_writes() {
        let installer = include_str!("../../../installer/file_converters/pipboy_art_manifest.cs");
        assert_eq!(
            installer_source::constant(installer, "ManifestFormat"),
            MANIFEST_FORMAT.to_string()
        );
    }

    #[test]
    fn the_loading_icon_clip_id_is_the_one_the_installer_exports() {
        let catalog = include_str!("../../../installer/file_converters/pipboy_art_catalog.cs");
        assert_eq!(
            installer_source::constant(catalog, "LoadingIconClipId"),
            FixedArt::LoadingIcon.clip_id()
        );
    }

    const SHARED_FRAME: &str = "shared.svg";

    fn clip(id: &str, frames: &[&str]) -> Value {
        json!({
            "id": id,
            "source": "Components\\VaultBoys\\Perks\\StrongBack.swf",
            "colour": "mask",
            "fps": 30.0,
            "script": [],
            "bounds": [0.0, 0.0, 10.0, 10.0],
            "frames": frames,
        })
    }

    fn fixed_clips() -> Vec<Value> {
        FixedArt::all()
            .map(|art| clip(&art.clip_id(), &[SHARED_FRAME]))
            .collect()
    }

    struct Fixture {
        dir: tempfile::TempDir,
    }

    impl Fixture {
        /// An asset root whose manifest holds the fixed clips plus `extra`, with `files`
        /// (relative to the art folder) present on disk.
        fn new(extra: Vec<Value>, files: &[&str]) -> Self {
            let mut clips = fixed_clips();
            clips.extend(extra);
            Self::with_manifest(&json!({ "format": 3, "clips": clips }).to_string(), files)
        }

        fn with_manifest(manifest: &str, files: &[&str]) -> Self {
            let dir = tempfile::tempdir().unwrap();
            let art = dir.path().join(INTERFACE_DIR).join(PIPBOY_DIR);
            for file in std::iter::once(&SHARED_FRAME).chain(files) {
                let path = art.join(file);
                std::fs::create_dir_all(path.parent().unwrap()).unwrap();
                std::fs::write(path, "<svg/>").unwrap();
            }
            std::fs::write(art.join(PIPBOY_ART_MANIFEST_FILE), manifest).unwrap();
            Self { dir }
        }

        fn load(&self) -> Result<ArtLibrary, ArtManifestError> {
            ArtLibrary::load(self.dir.path())
        }
    }

    #[test]
    fn a_valid_manifest_loads_and_answers_every_lookup() {
        let mut perk = clip(
            "vaultboy/components/vaultboys/perks/strongback",
            &["vb/0000.svg", "vb/0001.svg"],
        );
        perk["fps"] = json!(24.0);
        let mut card = clip("perkcard/0", &["card/0000.svg"]);
        card["source"] = json!("LevelUpMenu.swf");
        card["illustrates"] = json!("Components\\VaultBoys\\Perks\\StrongBack.swf");
        let fixture = Fixture::new(
            vec![perk, card],
            &["vb/0000.svg", "vb/0001.svg", "card/0000.svg"],
        );
        let library = fixture.load().unwrap();

        let vault_boy = library
            .clip_for_swf("interface/components/vaultboys/PERKS/strongback.SWF")
            .unwrap();
        assert_eq!(
            vault_boy.id,
            "vaultboy/components/vaultboys/perks/strongback"
        );
        assert!((vault_boy.fps - 24.0).abs() < f32::EPSILON);
        let card = library
            .perk_card_for_swf("Components/VaultBoys/Perks/StrongBack.swf")
            .unwrap();
        assert_eq!(card.id, "perkcard/0");
        assert!(library.clip_for_swf("Components\\Nope.swf").is_none());
        assert!(library.perk_card_for_swf("Components\\Nope.swf").is_none());
        assert!(library.clip("perkcard/0").is_some());
        assert!(library.clip("missing").is_none());
    }

    #[test]
    fn fixed_clips_resolve_and_frame_paths_clamp_to_the_last_frame() {
        let fixture = Fixture::new(
            vec![clip("extra", &["e/0000.svg", "e/0001.svg"])],
            &["e/0000.svg", "e/0001.svg"],
        );
        let library = fixture.load().unwrap();
        let fixed = library.fixed(FixedArt::Icon(Icon::Caps));
        assert_eq!(fixed.id, "icon/caps");
        let extra = library.clip("extra").unwrap();
        let art = fixture.dir.path().join(INTERFACE_DIR).join(PIPBOY_DIR);
        assert_eq!(library.frame_path(extra, 0), art.join("e/0000.svg"));
        assert_eq!(library.frame_path(extra, 1), art.join("e/0001.svg"));
        assert_eq!(library.frame_path(extra, 99), art.join("e/0001.svg"));
    }

    #[test]
    fn marker_types_without_a_clip_are_reported_once_each_in_order() {
        let fixture = Fixture::new(vec![clip("marker/11", &[SHARED_FRAME])], &[]);
        let library = fixture.load().unwrap();

        assert!(library.require_marker_clips([11, 11]).is_ok());
        assert!(matches!(
            library.require_marker_clips([30, 11, 12, 30]),
            Err(ArtManifestError::MissingMarkerClips(missing)) if missing == [12, 30]
        ));
    }

    #[test]
    fn a_clip_without_a_vaultboy_id_is_not_found_by_source() {
        let fixture = Fixture::new(vec![clip("marker/11", &[SHARED_FRAME])], &[]);
        let library = fixture.load().unwrap();
        assert!(
            library
                .clip_for_swf("Components\\VaultBoys\\Perks\\StrongBack.swf")
                .is_none()
        );
    }

    #[test]
    fn the_fixed_clip_list_has_every_role_once() {
        let ids: HashSet<String> = FixedArt::all().map(FixedArt::clip_id).collect();
        assert_eq!(
            ids.len(),
            Icon::ALL.len() + DollPiece::ALL.len() + 17 + 1 + 7 + 6
        );
    }

    #[test]
    fn a_missing_manifest_is_a_read_error() {
        let dir = tempfile::tempdir().unwrap();
        assert!(matches!(
            ArtLibrary::load(dir.path()),
            Err(ArtManifestError::Read { .. })
        ));
    }

    #[test]
    fn malformed_json_and_unknown_fields_are_parse_errors() {
        for manifest in [
            "{",
            r#"{"format":3,"clips":[],"extra":true}"#,
            r#"{"format":3,"clips":[{"id":"a"}]}"#,
        ] {
            let fixture = Fixture::with_manifest(manifest, &[]);
            assert!(
                matches!(fixture.load(), Err(ArtManifestError::Parse { .. })),
                "{manifest}"
            );
        }
    }

    #[test]
    fn another_format_version_is_unsupported() {
        let fixture = Fixture::with_manifest(r#"{"format":1,"clips":[]}"#, &[]);
        assert!(matches!(
            fixture.load(),
            Err(ArtManifestError::UnsupportedFormat { found: 1 })
        ));
    }

    #[test]
    fn a_repeated_id_is_rejected() {
        let fixture = Fixture::new(
            vec![clip("dup", &[SHARED_FRAME]), clip("dup", &[SHARED_FRAME])],
            &[],
        );
        assert!(matches!(
            fixture.load(),
            Err(ArtManifestError::DuplicateClip(id)) if id == "dup"
        ));
    }

    #[test]
    fn two_vault_boy_clips_from_one_movie_after_normalising_are_rejected() {
        let mut first = clip("vaultboy/a", &[SHARED_FRAME]);
        first["source"] = json!("Components\\VaultBoys\\Perks\\Same.swf");
        let mut second = clip("vaultboy/b", &[SHARED_FRAME]);
        second["source"] = json!("components/vaultboys/perks/SAME.swf");
        assert!(matches!(
            Fixture::new(vec![first, second], &[]).load(),
            Err(ArtManifestError::DuplicateSource { first, second, .. })
                if first == "vaultboy/a" && second == "vaultboy/b"
        ));
    }

    #[test]
    fn two_cards_illustrating_one_perk_movie_are_rejected() {
        let card = |id: &str, movie: &str| {
            let mut card = clip(id, &[SHARED_FRAME]);
            card["illustrates"] = json!(movie);
            card
        };
        let cards = vec![
            card("perkcard/0", "Components\\VaultBoys\\Perks\\Same.swf"),
            card("perkcard/1", "components/vaultboys/perks/same.swf"),
        ];
        assert!(matches!(
            Fixture::new(cards, &[]).load(),
            Err(ArtManifestError::DuplicateIllustration { first, second, .. })
                if first == "perkcard/0" && second == "perkcard/1"
        ));
    }

    #[test]
    fn marker_clips_may_share_a_source_movie() {
        let marker = |id: &str| {
            let mut marker = clip(id, &[SHARED_FRAME]);
            marker["source"] = json!("MapMenu.swf");
            marker
        };
        let fixture = Fixture::new(vec![marker("marker/1"), marker("marker/2")], &[]);
        assert!(fixture.load().is_ok());
    }

    fn invalid_reason(mutate: impl FnOnce(&mut Value)) -> &'static str {
        let mut bad = clip("bad", &[SHARED_FRAME]);
        mutate(&mut bad);
        match Fixture::new(vec![bad], &[]).load() {
            Err(ArtManifestError::InvalidClip { reason, .. }) => reason,
            other => panic!("expected InvalidClip, got {other:?}"),
        }
    }

    #[test]
    fn a_clip_script_is_parsed_and_drives_its_frame() {
        let mut card = clip("card", &["a.svg", "b.svg", "c.svg"]);
        card["script"] = json!([
            { "frame": 1, "action": "stop" },
            { "frame": 3, "action": "gotoAndPlay", "target": 2 },
        ]);
        let library = Fixture::new(vec![card], &["a.svg", "b.svg", "c.svg"])
            .load()
            .unwrap();
        let clip = library.clip("card").unwrap();

        assert_eq!(clip.frame_at(5.0, None), 0);
        assert_eq!(clip.frame_at(0.0, Some(PlaybackStart::PlayFrom(2))), 1);
    }

    #[test]
    fn a_missing_or_malformed_script_is_a_parse_error() {
        let mut absent = clip("bad", &[SHARED_FRAME]);
        absent.as_object_mut().unwrap().remove("script");
        let mut wrong_field = clip("bad", &[SHARED_FRAME]);
        wrong_field["script"] = json!([{ "frame": 1, "action": "stop", "extra": 1 }]);
        for bad in [absent, wrong_field] {
            assert!(matches!(
                Fixture::new(vec![bad], &[]).load(),
                Err(ArtManifestError::Parse { .. })
            ));
        }
    }

    #[test]
    fn invalid_scripts_are_rejected_naming_the_reason() {
        let script_reason = |script: Value| invalid_reason(|c| c["script"] = script);
        assert_eq!(
            script_reason(json!([{ "frame": 1, "action": "dance" }])),
            "a script step has an unknown action"
        );
        let needs_target = "a script step needs a target exactly when it jumps";
        assert_eq!(
            script_reason(json!([{ "frame": 1, "action": "gotoAndPlay" }])),
            needs_target
        );
        assert_eq!(
            script_reason(json!([{ "frame": 1, "action": "stop", "target": 1 }])),
            needs_target
        );
        let outside = "a script step names a frame outside the clip";
        assert_eq!(
            script_reason(json!([{ "frame": 2, "action": "stop" }])),
            outside
        );
        assert_eq!(
            script_reason(json!([{ "frame": 0, "action": "stop" }])),
            outside
        );
        assert_eq!(
            script_reason(json!([{ "frame": 1, "action": "gotoAndStop", "target": 2 }])),
            outside
        );
    }

    #[test]
    fn insane_clips_are_rejected_with_the_reason() {
        assert_eq!(
            invalid_reason(|c| c["frames"] = json!([])),
            "it has no frames"
        );
        assert_eq!(
            invalid_reason(|c| c["fps"] = json!(0.0)),
            "fps must be positive"
        );
        assert_eq!(
            invalid_reason(|c| c["fps"] = json!(-3.0)),
            "fps must be positive"
        );
        assert_eq!(
            invalid_reason(|c| c["bounds"] = json!([5.0, 0.0, 5.0, 10.0])),
            "the bounds are degenerate"
        );
        assert_eq!(
            invalid_reason(|c| c["bounds"] = json!([0.0, 9.0, 10.0, 1.0])),
            "the bounds are degenerate"
        );
        assert_eq!(invalid_reason(|c| c["id"] = json!("")), "the id is empty");
    }

    #[test]
    fn a_frame_path_leaving_the_art_folder_is_rejected() {
        for frame in ["../secret.svg", "/etc/passwd", "a/../../b.svg"] {
            let reason = invalid_reason(|c| c["frames"] = json!([frame]));
            assert_eq!(reason, "a frame path leaves the art folder", "{frame}");
        }
    }

    #[test]
    fn a_listed_frame_that_is_not_on_disk_is_reported_with_its_path() {
        let fixture = Fixture::new(vec![clip("gone", &["gone/0000.svg"])], &[]);
        match fixture.load() {
            Err(ArtManifestError::MissingFrame { clip, path }) => {
                assert_eq!(clip, "gone");
                assert!(path.ends_with("gone/0000.svg") || path.ends_with("gone\\0000.svg"));
            }
            other => panic!("expected MissingFrame, got {other:?}"),
        }
    }

    #[test]
    fn a_manifest_lacking_a_fixed_clip_names_it() {
        let mut clips = fixed_clips();
        clips.retain(|c| c["id"] != "special/luck");
        let manifest = json!({ "format": 3, "clips": clips }).to_string();
        let fixture = Fixture::with_manifest(&manifest, &[]);
        assert!(matches!(
            fixture.load(),
            Err(ArtManifestError::MissingFixedClip(id)) if id == "special/luck"
        ));
    }

    #[test]
    fn swf_paths_normalise_case_slashes_and_the_interface_prefix() {
        let expected = "components/vaultboys/perks/strongback.swf";
        for raw in [
            "Components\\VaultBoys\\Perks\\StrongBack.swf",
            "Interface\\Components\\VaultBoys\\Perks\\StrongBack.swf",
            "/interface/components/vaultboys/perks/strongback.swf",
            "  COMPONENTS/VAULTBOYS/PERKS/STRONGBACK.SWF ",
        ] {
            assert_eq!(normalize_swf_path(raw), expected, "{raw}");
        }
        assert_eq!(
            normalize_swf_path("Pipboy_InvPage.swf"),
            "pipboy_invpage.swf"
        );
    }

    #[test]
    fn a_placed_part_keeps_its_offset_and_tint() {
        let mut part = clip("extra", &[SHARED_FRAME]);
        part["offset"] = json!([1.25, 0.15]);
        part["tint"] = json!([0.3, 0.3, 0.3, 1.0]);
        let library = Fixture::new(vec![part], &[]).load().unwrap();
        let part = library.clip("extra").unwrap();
        assert_eq!(part.offset, Some([1.25, 0.15]));
        assert_eq!(part.tint, Some([0.3, 0.3, 0.3, 1.0]));
        assert_eq!(library.fixed(FixedArt::Icon(Icon::Caps)).offset, None);
        assert_eq!(library.fixed(FixedArt::Icon(Icon::Caps)).tint, None);
    }

    #[test]
    fn a_tint_with_a_negative_channel_is_rejected() {
        assert_eq!(
            invalid_reason(|c| c["tint"] = json!([0.3, -0.1, 0.3, 1.0])),
            "the tint must be finite and not negative"
        );
    }

    #[test]
    fn the_perk_grid_keys_are_distinct_texture_paths_below_the_textures_folder() {
        let keys: HashSet<&str> = PerkGridTexture::ALL
            .into_iter()
            .map(PerkGridTexture::key)
            .collect();
        assert_eq!(keys.len(), PerkGridTexture::ALL.len());
        assert!(
            keys.iter()
                .all(|key| key.starts_with("textures/interface/perkgrid/"))
        );
    }

    #[test]
    fn marker_clip_ids_use_the_raw_type() {
        assert_eq!(marker_clip_id(0), "marker/0");
        assert_eq!(marker_clip_id(58), "marker/58");
    }
}
