//! Chooses a large loading screen for a destination and keeps the one on show: its model, loaded
//! off the frame thread, and its tip.

use std::path::{Path, PathBuf};

use super::art_loader::{ArtLoadError, ArtLoader, LoaderPoll};
use crate::assets::db::DbClient;
use crate::assets::db::load_screens::{LoadScreen, LoadScreenCatalog};
use crate::assets::types::glb_key::glb_key;
use crate::loading_screen::{LoadScreenContext, Place, select};
use crate::random::SplitMix64;
use crate::rendering::loading_screen::{LoadedArt, LoadingArt, LoadingArtRenderer, ModelTransform};

/// Names the editor id of the loading screen to show on every large screen, to tune its camera.
pub(super) const FORCED_SCREEN_VAR: &str = "OPENFO4_LOADSCREEN";

/// The viewer may turn a screen that states no limits all the way around, like 442 of the 448
/// screens of the game do.
const DEFAULT_ROTATION_LIMITS_DEG: (i16, i16) = (-180, 180);

/// A screen picked for a destination, ready to be loaded.
pub(in crate::rendering) struct ChosenScreen {
    pub label: String,
    pub tip: Option<String>,
    pub art: LoadingArt,
}

/// What a frame's look at the model's load found.
pub(in crate::rendering) enum ArtStatus {
    /// No model is being loaded and none is shown.
    Idle,
    Pending,
    Ready,
    Failed {
        label: String,
        error: ArtLoadError,
    },
}

enum CatalogState {
    Ready(Box<LoadScreenCatalog>),
    /// The database predates the installer's loading screen export.
    Missing,
    Failed(String),
}

struct Showing {
    label: String,
    tip: Option<String>,
}

pub(in crate::rendering) struct LoadScreenHost {
    db_path: String,
    asset_root: PathBuf,
    catalog: Option<CatalogState>,
    rng: SplitMix64,
    forced: Option<String>,
    loader: ArtLoader,
    renderer: LoadingArtRenderer,
    device: wgpu::Device,
    queue: wgpu::Queue,
    pending: Option<Showing>,
    shown_tip: Option<String>,
}

impl LoadScreenHost {
    pub fn new(
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        target_format: wgpu::TextureFormat,
        asset_root: &Path,
        db_path: String,
        seed: u64,
    ) -> Self {
        Self {
            db_path,
            asset_root: asset_root.to_path_buf(),
            catalog: None,
            rng: SplitMix64::new(seed),
            forced: forced_screen_from_env(),
            loader: ArtLoader::default(),
            renderer: LoadingArtRenderer::new(device, queue, target_format),
            device: device.clone(),
            queue: queue.clone(),
            pending: None,
            shown_tip: None,
        }
    }

    /// Picks the large screen for `destination`, `cell` being the form key of its cell where it
    /// has one. `None`, after logging why, when the install has no screen to show.
    pub fn choose(&mut self, destination: &Place, cell: &str) -> Option<ChosenScreen> {
        let state = self
            .catalog
            .get_or_insert_with(|| load_catalog(&self.db_path));
        let catalog = match state {
            CatalogState::Ready(catalog) => catalog,
            CatalogState::Missing => {
                crate::utils::warn_once("load-screens-missing", || {
                    "the database has no loading screens (it predates their export, re-run the installer); large loading screens show the small one".to_string()
                });
                return None;
            }
            CatalogState::Failed(reason) => {
                tracing::error!("large loading screens are unavailable: {reason}");
                return None;
            }
        };
        let Some(screen) = pick(
            catalog,
            self.forced.as_deref(),
            destination,
            cell,
            &mut self.rng,
        ) else {
            tracing::warn!(
                "no loading screen is eligible for {destination:?}; showing the small one"
            );
            return None;
        };
        Some(ChosenScreen {
            label: screen.label().to_string(),
            tip: screen.tip.clone(),
            art: art_for(screen, &self.asset_root),
        })
    }

    /// Starts loading `chosen`'s model; what was shown before goes. The tip waits for the model.
    pub fn show(&mut self, chosen: ChosenScreen) -> Result<(), ArtLoadError> {
        self.clear();
        self.loader.start(chosen.art)?;
        self.pending = Some(Showing {
            label: chosen.label,
            tip: chosen.tip,
        });
        Ok(())
    }

    /// Takes down whatever is shown or loading.
    pub fn clear(&mut self) {
        self.loader.cancel();
        self.renderer.set_art(None);
        self.pending = None;
        self.shown_tip = None;
    }

    /// Whether a model is shown or loading.
    pub fn is_active(&self) -> bool {
        self.pending.is_some() || self.renderer.has_art()
    }

    /// Moves a finished model onto the GPU and starts showing it.
    pub fn poll(&mut self) -> ArtStatus {
        let result = match self.loader.poll() {
            LoaderPoll::Idle => return ArtStatus::Idle,
            LoaderPoll::Pending => return ArtStatus::Pending,
            LoaderPoll::Done(result) => result,
        };
        let Some(showing) = self.pending.take() else {
            return ArtStatus::Idle;
        };
        let loaded = result.and_then(|cpu| {
            LoadedArt::upload(&self.device, &self.queue, cpu).map_err(ArtLoadError::from)
        });
        match loaded {
            Ok(loaded) => {
                self.renderer.set_art(Some(loaded));
                self.shown_tip = showing.tip;
                ArtStatus::Ready
            }
            Err(error) => ArtStatus::Failed {
                label: showing.label,
                error,
            },
        }
    }

    pub fn has_art(&self) -> bool {
        self.renderer.has_art()
    }

    /// The tip of the screen on show; none until its model is.
    pub fn tip(&self) -> Option<&str> {
        self.shown_tip
            .as_deref()
            .filter(|_| self.renderer.has_art())
    }

    pub fn drag(&mut self, dx_px: f32) {
        self.renderer.drag(dx_px);
    }

    pub fn wheel(&mut self, lines: f32) {
        self.renderer.wheel(lines);
    }

    pub fn render(
        &mut self,
        encoder: &mut wgpu::CommandEncoder,
        view: &wgpu::TextureView,
        size: (u32, u32),
    ) {
        self.renderer.render(encoder, view, size);
    }
}

fn load_catalog(db_path: &str) -> CatalogState {
    match DbClient::new(db_path).and_then(|db| db.get_load_screen_catalog()) {
        Ok(Some(catalog)) => CatalogState::Ready(Box::new(catalog)),
        Ok(None) => CatalogState::Missing,
        Err(error) => CatalogState::Failed(format!("{error:#}")),
    }
}

fn forced_screen_from_env() -> Option<String> {
    match std::env::var(FORCED_SCREEN_VAR) {
        Ok(editor_id) if !editor_id.is_empty() => Some(editor_id),
        Ok(_) | Err(std::env::VarError::NotPresent) => None,
        Err(error @ std::env::VarError::NotUnicode(_)) => {
            tracing::error!("{FORCED_SCREEN_VAR} is ignored: {error}");
            None
        }
    }
}

/// The screen to show at `destination`: the forced one where it exists, else the selection
/// rule's. Each draws from a generator of its own, forked from `rng`.
fn pick<'a>(
    catalog: &'a LoadScreenCatalog,
    forced: Option<&str>,
    destination: &Place,
    cell: &str,
    rng: &mut SplitMix64,
) -> Option<&'a LoadScreen> {
    if let Some(editor_id) = forced {
        let found = catalog.screens.iter().find(|screen| {
            screen
                .editor_id
                .as_deref()
                .is_some_and(|own| own.eq_ignore_ascii_case(editor_id))
        });
        if found.is_some() {
            return found;
        }
        tracing::error!(
            "{FORCED_SCREEN_VAR}={editor_id} names no loading screen; choosing as usual"
        );
    }
    let context = LoadScreenContext::new(catalog, destination, cell, rng.fork());
    select(catalog, &context, &mut rng.fork())
}

/// The model and its limits as the renderer takes them. A screen without zoom limits of its own
/// zooms within its transform's.
fn art_for(screen: &LoadScreen, asset_root: &Path) -> LoadingArt {
    let transform = &screen.transform;
    LoadingArt {
        model_path: asset_root.join(glb_key(&screen.model_path)),
        asset_root: asset_root.to_path_buf(),
        transform: ModelTransform {
            position: transform.position,
            rotation_radians: transform.rotation_radians,
            scale: transform.scale,
            around_origin: transform.around_origin,
        },
        rotation_limits_deg: screen
            .rotation_limits
            .unwrap_or(DEFAULT_ROTATION_LIMITS_DEG),
        zoom_limits: screen
            .zoom_limits
            .unwrap_or((transform.zoom_min, transform.zoom_max)),
        no_rotation: screen.no_rotation,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::assets::db::load_screens::ModelTransform as RecordTransform;

    fn screen(editor_id: &str) -> LoadScreen {
        LoadScreen {
            form_key: format!("0001:{editor_id}"),
            editor_id: Some(editor_id.to_string()),
            tip: Some(format!("tip of {editor_id}")),
            model_path: format!("Props\\{editor_id}.nif"),
            displays_in_main_menu: false,
            no_rotation: false,
            transform: RecordTransform {
                position: [1.0, 2.0, 3.0],
                rotation_radians: [0.1, 0.2, 0.3],
                scale: 1.5,
                around_origin: true,
                zoom_min: -0.25,
                zoom_max: 0.75,
            },
            rotation_limits: Some((-90, 45)),
            zoom_limits: Some((-0.5, 1.5)),
            conditions: Vec::new(),
        }
    }

    fn catalog(editor_ids: &[&str]) -> LoadScreenCatalog {
        LoadScreenCatalog {
            screens: editor_ids.iter().map(|id| screen(id)).collect(),
            ..LoadScreenCatalog::default()
        }
    }

    fn commonwealth() -> Place {
        Place::Exterior {
            worldspace: "0001:Commonwealth".to_string(),
        }
    }

    #[test]
    fn the_art_names_the_converted_model_below_the_asset_root() {
        let art = art_for(&screen("Crate"), Path::new("root"));
        assert_eq!(
            art.model_path,
            Path::new("root").join("meshes\\Props\\Crate.glb")
        );
        assert_eq!(art.asset_root, Path::new("root"));
    }

    #[test]
    fn the_art_carries_the_placement_and_the_limits_of_the_record() {
        let art = art_for(&screen("Crate"), Path::new("root"));
        assert_eq!(art.transform.position, [1.0, 2.0, 3.0]);
        assert_eq!(art.transform.rotation_radians, [0.1, 0.2, 0.3]);
        assert_eq!(art.transform.scale, 1.5);
        assert!(art.transform.around_origin);
        assert_eq!(art.rotation_limits_deg, (-90, 45));
        assert_eq!(art.zoom_limits, (-0.5, 1.5));
        assert!(!art.no_rotation);
    }

    #[test]
    fn a_screen_without_zoom_limits_zooms_within_its_transforms() {
        let mut record = screen("Crate");
        record.zoom_limits = None;
        let art = art_for(&record, Path::new("root"));
        assert_eq!(art.zoom_limits, (-0.25, 0.75));
    }

    #[test]
    fn a_screen_without_rotation_limits_turns_all_the_way_around() {
        let mut record = screen("Crate");
        record.rotation_limits = None;
        let art = art_for(&record, Path::new("root"));
        assert_eq!(art.rotation_limits_deg, (-180, 180));
    }

    #[test]
    fn the_no_rotation_flag_reaches_the_art() {
        let mut record = screen("Crate");
        record.no_rotation = true;
        assert!(art_for(&record, Path::new("root")).no_rotation);
    }

    #[test]
    fn a_forced_screen_is_found_without_regard_to_case() {
        let catalog = catalog(&["Alpha", "Bravo"]);
        let mut rng = SplitMix64::new(1);
        for _ in 0..20 {
            let picked = pick(&catalog, Some("bRaVo"), &commonwealth(), "", &mut rng);
            assert_eq!(picked.map(LoadScreen::label), Some("Bravo"));
        }
    }

    #[test]
    fn an_unknown_forced_screen_falls_back_to_the_selection_rule() {
        let catalog = catalog(&["Alpha"]);
        let mut rng = SplitMix64::new(1);
        let picked = pick(&catalog, Some("Nope"), &commonwealth(), "", &mut rng);
        assert_eq!(picked.map(LoadScreen::label), Some("Alpha"));
    }

    #[test]
    fn selection_draws_from_the_catalog_and_is_deterministic_for_a_seed() {
        let catalog = catalog(&["Alpha", "Bravo", "Charlie", "Delta"]);
        let draw = |seed| {
            let mut rng = SplitMix64::new(seed);
            (0..16)
                .map(|_| {
                    pick(&catalog, None, &commonwealth(), "", &mut rng)
                        .expect("every screen is eligible")
                        .label()
                        .to_string()
                })
                .collect::<Vec<_>>()
        };
        assert_eq!(draw(7), draw(7));
        let first = draw(7);
        assert!(
            first.iter().any(|label| label != &first[0]),
            "the pick varies between draws: {first:?}"
        );
    }

    #[test]
    fn an_empty_catalog_has_nothing_to_pick() {
        let mut rng = SplitMix64::new(1);
        assert!(pick(&catalog(&[]), None, &Place::Interior, "", &mut rng).is_none());
    }

    mod on_the_gpu {
        use std::time::{Duration, Instant};

        use super::super::*;
        use crate::rendering::gpu_test::test_device;
        use crate::rendering::loading_screen::test_support::cube_art;
        use crate::rendering::post::tonemap::ui_gamma_format;

        const FORMAT: wgpu::TextureFormat = wgpu::TextureFormat::Rgba8UnormSrgb;
        const WAIT: Duration = Duration::from_secs(20);

        fn host(test: &str) -> Option<LoadScreenHost> {
            let (device, queue) = test_device(test)?;
            Some(LoadScreenHost::new(
                &device,
                &queue,
                ui_gamma_format(FORMAT),
                Path::new("unused"),
                String::new(),
                1,
            ))
        }

        fn chosen(art: LoadingArt) -> ChosenScreen {
            ChosenScreen {
                label: "TestScreen".to_string(),
                tip: Some("a tip".to_string()),
                art,
            }
        }

        fn settled(host: &mut LoadScreenHost) -> ArtStatus {
            let deadline = Instant::now() + WAIT;
            loop {
                match host.poll() {
                    ArtStatus::Pending => {
                        assert!(Instant::now() < deadline, "the load did not finish");
                        std::thread::sleep(Duration::from_millis(2));
                    }
                    status => return status,
                }
            }
        }

        #[test]
        fn the_tip_shows_once_the_model_has_arrived_and_not_before() {
            let Some(mut host) = host("load screen host ready") else {
                return;
            };
            let dir = tempfile::tempdir().unwrap();
            host.show(chosen(cube_art(dir.path(), ""))).unwrap();
            assert!(host.is_active());
            assert_eq!(host.tip(), None, "the model is still loading");
            assert!(matches!(settled(&mut host), ArtStatus::Ready));
            assert!(host.has_art());
            assert_eq!(host.tip(), Some("a tip"));
        }

        #[test]
        fn a_model_that_fails_reports_its_screen_and_leaves_nothing_shown() {
            let Some(mut host) = host("load screen host failed") else {
                return;
            };
            let dir = tempfile::tempdir().unwrap();
            let mut art = cube_art(dir.path(), "");
            art.model_path = dir.path().join("no_such_model.glb");
            host.show(chosen(art)).unwrap();
            let ArtStatus::Failed { label, error } = settled(&mut host) else {
                panic!("the load must fail");
            };
            assert_eq!(label, "TestScreen");
            assert!(error.to_string().contains("no_such_model.glb"), "{error}");
            assert!(!host.has_art());
            assert_eq!(host.tip(), None);
            assert!(!host.is_active());
        }

        #[test]
        fn clearing_takes_down_a_shown_screen_and_one_still_loading() {
            let Some(mut host) = host("load screen host clear") else {
                return;
            };
            let dir = tempfile::tempdir().unwrap();
            host.show(chosen(cube_art(dir.path(), ""))).unwrap();
            assert!(matches!(settled(&mut host), ArtStatus::Ready));
            host.clear();
            assert!(!host.is_active());
            assert_eq!(host.tip(), None);

            host.show(chosen(cube_art(dir.path(), ""))).unwrap();
            host.clear();
            assert!(!host.is_active());
            assert!(
                matches!(host.poll(), ArtStatus::Idle),
                "the load is forgotten"
            );
        }

        #[test]
        fn a_new_screen_replaces_the_one_before() {
            let Some(mut host) = host("load screen host replace") else {
                return;
            };
            let dir = tempfile::tempdir().unwrap();
            host.show(chosen(cube_art(dir.path(), ""))).unwrap();
            assert!(matches!(settled(&mut host), ArtStatus::Ready));
            let mut second = chosen(cube_art(dir.path(), ""));
            second.tip = Some("another tip".to_string());
            host.show(second).unwrap();
            assert_eq!(host.tip(), None, "the first screen's tip went with it");
            assert!(matches!(settled(&mut host), ArtStatus::Ready));
            assert_eq!(host.tip(), Some("another tip"));
        }
    }
}
