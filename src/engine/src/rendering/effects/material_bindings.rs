//! The textures and samplers an effect material binds (group 2 of the effect pipeline layout),
//! loaded from the asset root on first use and cached per material.

use std::collections::HashMap;
use std::sync::{Arc, Mutex, Weak};

use super::lifetime::material_id;
use super::shader::bindings;
use crate::assets::layout::{TEXTURES_DIR, texture_key};
use crate::assets::types::effect_material::{ClampMode, EffectMaterial, EffectTextures};
use crate::assets::types::texture::TextureError;
use crate::rendering::texture::load_dds_view_as;

const WHITE_TEXEL: [u8; 4] = [255; 4];
const MATERIAL_SAMPLER_ANISOTROPY: u16 = 4;
const DATA_PREFIX: &str = "data\\";

#[derive(Debug, thiserror::Error)]
#[error("effect texture {path}: {source}")]
pub struct EffectTextureError {
    pub path: String,
    #[source]
    pub source: TextureError,
}

/// Whether the shader decodes a texture's colour itself (linear) or expects the view to do it.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub(crate) enum ColourSpace {
    Srgb,
    Linear,
}

/// A material's texture path below the textures folder: slashes normalised and a leading
/// `data\` and `textures\` dropped.
pub fn path_below_textures(stored: &str) -> String {
    let normalised = stored.trim().replace('/', "\\");
    let relative = normalised
        .get(..DATA_PREFIX.len())
        .filter(|head| head.eq_ignore_ascii_case(DATA_PREFIX))
        .map_or(normalised.as_str(), |_| &normalised[DATA_PREFIX.len()..]);
    match relative.split_once('\\') {
        Some((root, below_root)) if root.eq_ignore_ascii_case(TEXTURES_DIR) => {
            below_root.to_owned()
        }
        _ => relative.to_owned(),
    }
}

/// The asset-root-relative path of a material's texture.
pub fn effect_texture_path(stored: &str) -> String {
    texture_key(&path_below_textures(stored))
}

/// How the material's textures map onto the slots of the pipeline layout.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum SlotSource {
    Source,
    Normal,
    Greyscale,
    EnvironmentMask,
    /// Not emitted by any supported permutation: bound to the neutral texture.
    Unused,
    /// The environment cube map: the cube view the stage resolved through the shared
    /// `CubeArray` (its black cube when the material reflects nothing).
    EnvironmentCube,
}

const fn slot_source(slot: u32) -> (SlotSource, ColourSpace) {
    match slot {
        0 => (SlotSource::Source, ColourSpace::Srgb),
        1 => (SlotSource::Normal, ColourSpace::Linear),
        4 => (SlotSource::Greyscale, ColourSpace::Srgb),
        5 => (SlotSource::EnvironmentCube, ColourSpace::Srgb),
        7 => (SlotSource::EnvironmentMask, ColourSpace::Linear),
        _ => (SlotSource::Unused, ColourSpace::Linear),
    }
}

/// The samplers of the four texture clamp modes, plus the fixed clamp samplers.
struct Samplers {
    by_clamp: [wgpu::Sampler; 4],
    clamp: wgpu::Sampler,
}

impl Samplers {
    fn new(device: &wgpu::Device) -> Self {
        use wgpu::AddressMode::{ClampToEdge as C, Repeat as R};
        let make = |u: wgpu::AddressMode, v: wgpu::AddressMode, anisotropy: u16| {
            device.create_sampler(&wgpu::SamplerDescriptor {
                label: Some("effect_sampler"),
                address_mode_u: u,
                address_mode_v: v,
                address_mode_w: wgpu::AddressMode::ClampToEdge,
                mag_filter: wgpu::FilterMode::Linear,
                min_filter: wgpu::FilterMode::Linear,
                mipmap_filter: wgpu::MipmapFilterMode::Linear,
                anisotropy_clamp: anisotropy,
                ..Default::default()
            })
        };
        Self {
            by_clamp: [
                make(C, C, MATERIAL_SAMPLER_ANISOTROPY),
                make(C, R, MATERIAL_SAMPLER_ANISOTROPY),
                make(R, C, MATERIAL_SAMPLER_ANISOTROPY),
                make(R, R, MATERIAL_SAMPLER_ANISOTROPY),
            ],
            clamp: make(C, C, 1),
        }
    }

    fn material(&self, mode: ClampMode) -> &wgpu::Sampler {
        &self.by_clamp[match mode {
            ClampMode::ClampSClampT => 0,
            ClampMode::ClampSWrapT => 1,
            ClampMode::WrapSClampT => 2,
            ClampMode::WrapSWrapT => 3,
        }]
    }

    /// Slots `s0`, `s1` and `s7` follow the material's clamp mode; the others clamp.
    fn for_slot(&self, slot: u32, mode: ClampMode) -> &wgpu::Sampler {
        if matches!(slot, 0 | 1 | 7) {
            self.material(mode)
        } else {
            &self.clamp
        }
    }
}

struct Neutral {
    flat: wgpu::TextureView,
}

impl Neutral {
    fn new(device: &wgpu::Device, queue: &wgpu::Queue) -> Self {
        use wgpu::util::DeviceExt;
        let texture = device.create_texture_with_data(
            queue,
            &wgpu::TextureDescriptor {
                label: Some("effect_neutral_texture"),
                size: wgpu::Extent3d {
                    width: 1,
                    height: 1,
                    depth_or_array_layers: 1,
                },
                mip_level_count: 1,
                sample_count: 1,
                dimension: wgpu::TextureDimension::D2,
                format: wgpu::TextureFormat::Rgba8Unorm,
                usage: wgpu::TextureUsages::TEXTURE_BINDING,
                view_formats: &[],
            },
            wgpu::util::TextureDataOrder::LayerMajor,
            &WHITE_TEXEL,
        );
        Self {
            flat: texture.create_view(&wgpu::TextureViewDescriptor::default()),
        }
    }
}

/// Texture views by asset-root-relative path and colour space, loaded on first use. The cache
/// holds each view weakly: a view stays alive for as long as a bind group built from it does and
/// is forgotten (and loaded again on demand) once the last of them is dropped. Shared with a
/// warm-up worker thread: the lock is not held while a file is read.
pub(crate) struct TextureCache {
    device: wgpu::Device,
    queue: wgpu::Queue,
    asset_root: String,
    views: Mutex<HashMap<(String, ColourSpace), Weak<wgpu::TextureView>>>,
}

impl TextureCache {
    pub fn new(device: &wgpu::Device, queue: &wgpu::Queue, asset_root: &str) -> Self {
        Self {
            device: device.clone(),
            queue: queue.clone(),
            asset_root: asset_root.to_owned(),
            views: Mutex::new(HashMap::new()),
        }
    }

    /// The view of the texture at `path`; a missing or unreadable file is an error.
    pub fn view(
        &self,
        path: &str,
        space: ColourSpace,
    ) -> Result<Arc<wgpu::TextureView>, EffectTextureError> {
        let key = (path.to_lowercase(), space);
        if let Some(view) = self.lock().get(&key).and_then(Weak::upgrade) {
            return Ok(view);
        }
        let srgb = space == ColourSpace::Srgb;
        let view = load_dds_view_as(&self.device, &self.queue, &self.asset_root, path, srgb)
            .map(Arc::new)
            .map_err(|source| EffectTextureError {
                path: path.to_owned(),
                source,
            })?;
        self.lock().insert(key, Arc::downgrade(&view));
        Ok(view)
    }

    /// Forgets the views nothing holds any more.
    pub fn prune(&self) {
        self.lock().retain(|_, view| view.strong_count() > 0);
    }

    pub fn count(&self) -> usize {
        self.lock().len()
    }

    fn lock(
        &self,
    ) -> std::sync::MutexGuard<'_, HashMap<(String, ColourSpace), Weak<wgpu::TextureView>>> {
        self.views
            .lock()
            .expect("the texture cache lock is not poisoned")
    }
}

/// A bind group and the cached texture views it reads, which keep those views cached.
pub struct BuiltGroup {
    pub group: wgpu::BindGroup,
    pub textures: Vec<Arc<wgpu::TextureView>>,
}

struct CachedGroup<M> {
    material: Weak<M>,
    built: BuiltGroup,
}

/// The finished bind groups of the materials in use, one per material.
pub struct GroupCache<M> {
    groups: HashMap<usize, CachedGroup<M>>,
}

impl<M> Default for GroupCache<M> {
    fn default() -> Self {
        Self {
            groups: HashMap::new(),
        }
    }
}

impl<M> GroupCache<M> {
    pub fn insert(&mut self, material: &Arc<M>, built: BuiltGroup) {
        self.groups.insert(
            material_id(material),
            CachedGroup {
                material: Arc::downgrade(material),
                built,
            },
        );
    }

    pub fn get(&self, material: &Arc<M>) -> Option<&wgpu::BindGroup> {
        self.groups
            .get(&material_id(material))
            .filter(|cached| cached.material.strong_count() > 0)
            .map(|cached| &cached.built.group)
    }

    /// Drops the bind groups of materials that nothing holds any more.
    pub fn release_unused(&mut self) {
        self.groups
            .retain(|_, cached| cached.material.strong_count() > 0);
    }

    pub fn count(&self) -> usize {
        self.groups.len()
    }
}

/// A view a bind group slot shows: a permanent one, or one shared through the texture cache.
enum SlotView {
    Permanent(wgpu::TextureView),
    Cached(Arc<wgpu::TextureView>),
}

impl SlotView {
    fn view(&self) -> &wgpu::TextureView {
        match self {
            Self::Permanent(view) => view,
            Self::Cached(view) => view,
        }
    }
}

/// Loads a material's textures and builds its bind group. Shared with the warm-up worker
/// thread.
pub struct MaterialLoader {
    device: wgpu::Device,
    layout: wgpu::BindGroupLayout,
    textures: TextureCache,
    neutral: Neutral,
    samplers: Samplers,
}

impl MaterialLoader {
    /// The bind group of `material`, with `environment_cube` at `t5`. A missing or unreadable
    /// texture the material names is an error: the converter and the deployed files disagree.
    pub fn build_group(
        &self,
        material: &EffectMaterial,
        environment_cube: &wgpu::TextureView,
    ) -> Result<BuiltGroup, EffectTextureError> {
        let views: Vec<(u32, SlotView)> = bindings::MATERIAL_TEXTURE_SLOTS
            .iter()
            .map(|&slot| {
                self.view_for(slot, &material.textures, environment_cube)
                    .map(|view| (slot, view))
            })
            .collect::<Result<_, _>>()?;
        let entries: Vec<wgpu::BindGroupEntry> = views
            .iter()
            .flat_map(|(slot, view)| {
                [
                    wgpu::BindGroupEntry {
                        binding: bindings::texture_binding(*slot),
                        resource: wgpu::BindingResource::TextureView(view.view()),
                    },
                    wgpu::BindGroupEntry {
                        binding: bindings::sampler_binding(*slot),
                        resource: wgpu::BindingResource::Sampler(
                            self.samplers.for_slot(*slot, material.clamp),
                        ),
                    },
                ]
            })
            .collect();
        let group = self.device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("effect_material_bind_group"),
            layout: &self.layout,
            entries: &entries,
        });
        drop(entries);
        let textures = views
            .into_iter()
            .filter_map(|(_, view)| match view {
                SlotView::Cached(view) => Some(view),
                SlotView::Permanent(_) => None,
            })
            .collect();
        Ok(BuiltGroup { group, textures })
    }

    fn view_for(
        &self,
        slot: u32,
        textures: &EffectTextures,
        environment_cube: &wgpu::TextureView,
    ) -> Result<SlotView, EffectTextureError> {
        let (source, space) = slot_source(slot);
        let stored = match source {
            SlotSource::Source => &textures.source,
            SlotSource::Normal => &textures.normal,
            SlotSource::Greyscale => &textures.greyscale,
            SlotSource::EnvironmentMask => &textures.env_mask,
            SlotSource::EnvironmentCube => {
                return Ok(SlotView::Permanent(environment_cube.clone()));
            }
            SlotSource::Unused => return Ok(SlotView::Permanent(self.neutral.flat.clone())),
        };
        let Some(stored) = EffectTextures::used(stored) else {
            return Ok(SlotView::Permanent(self.neutral.flat.clone()));
        };
        self.textures
            .view(&effect_texture_path(stored), space)
            .map(SlotView::Cached)
    }
}

/// The finished bind groups of the effect materials in use, one per material.
pub struct MaterialBindings {
    loader: Arc<MaterialLoader>,
    groups: GroupCache<EffectMaterial>,
}

impl MaterialBindings {
    pub fn new(
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        layout: &wgpu::BindGroupLayout,
        asset_root: &str,
    ) -> Self {
        Self {
            loader: Arc::new(MaterialLoader {
                device: device.clone(),
                layout: layout.clone(),
                textures: TextureCache::new(device, queue, asset_root),
                neutral: Neutral::new(device, queue),
                samplers: Samplers::new(device),
            }),
            groups: GroupCache::default(),
        }
    }

    pub fn loader(&self) -> Arc<MaterialLoader> {
        Arc::clone(&self.loader)
    }

    /// Builds the bind group of `material` on this thread if it has none.
    pub fn ensure(
        &mut self,
        material: &Arc<EffectMaterial>,
        environment_cube: &wgpu::TextureView,
    ) -> Result<(), EffectTextureError> {
        if self.group(material).is_none() {
            let built = self.loader.build_group(material, environment_cube)?;
            self.insert(material, built);
        }
        Ok(())
    }

    /// Stores a bind group another thread built for `material`.
    pub fn insert(&mut self, material: &Arc<EffectMaterial>, built: BuiltGroup) {
        self.groups.insert(material, built);
    }

    pub fn group(&self, material: &Arc<EffectMaterial>) -> Option<&wgpu::BindGroup> {
        self.groups.get(material)
    }

    /// Drops the bind groups of materials that no placement or particle system holds any more,
    /// and with them the texture views no remaining group reads.
    pub fn release_unused(&mut self) {
        self.groups.release_unused();
        self.loader.textures.prune();
    }

    pub fn group_count(&self) -> usize {
        self.groups.count()
    }

    pub fn cached_texture_count(&self) -> usize {
        self.loader.textures.count()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::rendering::cube_array::{CubeArray, CubeHandle};
    use crate::rendering::effects::pipeline::EffectLayouts;
    use crate::rendering::effects::test_fixtures::stub_material;
    use crate::rendering::{gpu_test::test_device, scene_copy::SceneCopy};

    #[test]
    fn stored_paths_resolve_below_the_asset_root_textures_folder() {
        assert_eq!(
            effect_texture_path("effects/embers_d.dds"),
            "textures\\effects\\embers_d.dds"
        );
        assert_eq!(
            effect_texture_path("Data\\Textures\\Effects\\Gradients\\fire.dds"),
            "textures\\Effects\\Gradients\\fire.dds"
        );
        assert_eq!(
            effect_texture_path("textures/effects/x.dds"),
            "textures\\effects\\x.dds"
        );
    }

    #[test]
    fn the_slot_table_follows_the_documented_slot_contents() {
        assert_eq!(slot_source(0), (SlotSource::Source, ColourSpace::Srgb));
        assert_eq!(slot_source(1), (SlotSource::Normal, ColourSpace::Linear));
        assert_eq!(slot_source(4), (SlotSource::Greyscale, ColourSpace::Srgb));
        assert_eq!(
            slot_source(5),
            (SlotSource::EnvironmentCube, ColourSpace::Srgb)
        );
        assert_eq!(
            slot_source(7),
            (SlotSource::EnvironmentMask, ColourSpace::Linear)
        );
        for slot in [2, 6, 8] {
            assert_eq!(slot_source(slot).0, SlotSource::Unused);
        }
    }

    fn black_cube(device: &wgpu::Device, queue: &wgpu::Queue) -> wgpu::TextureView {
        let cubes = CubeArray::new(device, queue, std::path::Path::new("unused-root"));
        cubes.view(CubeHandle::NONE).clone()
    }

    #[test]
    fn stored_paths_split_below_the_textures_folder() {
        assert_eq!(
            path_below_textures("Data\\Textures\\Shared\\Cube.dds"),
            "Shared\\Cube.dds"
        );
        assert_eq!(path_below_textures("shared/cube.dds"), "shared\\cube.dds");
    }

    fn fixture(device: &wgpu::Device) -> (SceneCopy, EffectLayouts) {
        let scene = SceneCopy::new(device, 4, 4);
        let layouts = EffectLayouts::new(device, scene.layout());
        (scene, layouts)
    }

    #[test]
    fn a_material_without_textures_gets_a_group_over_the_neutral_textures() {
        let Some((device, queue)) = test_device("effect material bindings neutral") else {
            return;
        };
        let (_scene, layouts) = fixture(&device);
        let mut bindings = MaterialBindings::new(&device, &queue, &layouts.material, "unused-root");
        let material = Arc::new(stub_material());
        let cube = black_cube(&device, &queue);

        let scope = device.push_error_scope(wgpu::ErrorFilter::Validation);
        bindings.ensure(&material, &cube).unwrap();
        bindings.ensure(&material, &cube).unwrap();

        assert!(bindings.group(&material).is_some());
        assert_eq!(bindings.group_count(), 1);
        assert!(pollster::block_on(scope.pop()).is_none());
    }

    #[test]
    fn a_missing_texture_is_an_error_naming_its_path() {
        let Some((device, queue)) = test_device("effect material bindings missing") else {
            return;
        };
        let (_scene, layouts) = fixture(&device);
        let mut bindings =
            MaterialBindings::new(&device, &queue, &layouts.material, "no-such-asset-root");
        let mut material = stub_material();
        material.textures.source = "effects/missing_d.dds".to_owned();
        let material = Arc::new(material);

        let error = bindings
            .ensure(&material, &black_cube(&device, &queue))
            .unwrap_err();

        assert_eq!(error.path, "textures\\effects\\missing_d.dds");
        assert!(bindings.group(&material).is_none());
    }

    #[test]
    fn the_group_of_a_dropped_material_is_released() {
        let Some((device, queue)) = test_device("effect material bindings release") else {
            return;
        };
        let (_scene, layouts) = fixture(&device);
        let mut bindings = MaterialBindings::new(&device, &queue, &layouts.material, "unused-root");
        let kept = Arc::new(stub_material());
        let dropped = Arc::new(stub_material());
        let cube = black_cube(&device, &queue);
        for material in [&kept, &dropped] {
            bindings.ensure(material, &cube).unwrap();
        }

        drop(dropped);
        bindings.release_unused();

        assert_eq!(bindings.group_count(), 1);
        assert!(bindings.group(&kept).is_some());
    }

    fn root_with_texture(name: &str) -> tempfile::TempDir {
        use crate::assets::types::texture::cube_test_support::{DdsSpec, Format, Header};
        let root = tempfile::TempDir::new().unwrap();
        let textures = root.path().join("textures").join("effects");
        std::fs::create_dir_all(&textures).unwrap();
        let spec = DdsSpec {
            cube: false,
            face_flags: false,
            ..DdsSpec::cube(4, 1, Format::Rgba8, Header::Legacy)
        };
        spec.write(&textures, name);
        root
    }

    fn textured_material(name: &str) -> Arc<EffectMaterial> {
        let mut material = stub_material();
        material.textures.source = format!("effects/{name}");
        Arc::new(material)
    }

    #[test]
    fn a_texture_stays_cached_while_a_live_material_reads_it_and_goes_with_the_last_one() {
        let Some((device, queue)) = test_device("effect material bindings eviction") else {
            return;
        };
        let (_scene, layouts) = fixture(&device);
        let root = root_with_texture("shared_d.dds");
        let root_path = root.path().to_str().unwrap();
        let mut bindings = MaterialBindings::new(&device, &queue, &layouts.material, root_path);
        let first = textured_material("shared_d.dds");
        let second = textured_material("shared_d.dds");
        let cube = black_cube(&device, &queue);
        bindings.ensure(&first, &cube).unwrap();
        bindings.ensure(&second, &cube).unwrap();
        assert_eq!(bindings.cached_texture_count(), 1);

        drop(first);
        bindings.release_unused();
        assert_eq!(
            bindings.cached_texture_count(),
            1,
            "the second still reads it"
        );

        drop(second);
        bindings.release_unused();
        assert_eq!(bindings.cached_texture_count(), 0);
    }

    #[test]
    fn an_evicted_texture_is_loaded_again_when_a_material_needs_it() {
        let Some((device, queue)) = test_device("effect material bindings reload") else {
            return;
        };
        let (_scene, layouts) = fixture(&device);
        let root = root_with_texture("again_d.dds");
        let root_path = root.path().to_str().unwrap();
        let mut bindings = MaterialBindings::new(&device, &queue, &layouts.material, root_path);
        let cube = black_cube(&device, &queue);
        bindings
            .ensure(&textured_material("again_d.dds"), &cube)
            .unwrap();
        bindings.release_unused();
        assert_eq!(bindings.cached_texture_count(), 0);

        let again = textured_material("again_d.dds");
        bindings.ensure(&again, &cube).unwrap();

        assert!(bindings.group(&again).is_some());
        assert_eq!(bindings.cached_texture_count(), 1);
    }
}
