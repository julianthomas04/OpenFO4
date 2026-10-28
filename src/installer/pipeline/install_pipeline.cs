using AssetConverter.Progress;

namespace AssetConverter.Pipeline
{
    /// <summary>
    /// The full install as concurrent lanes joined by gates (the diagram is in docs/installer/README.md, "Pipeline"). A lane starts
    /// the moment the gates and lanes it reads from are done; <see cref="RunAsync"/> declares every lane with its dependencies, and
    /// <see cref="PipelineStages"/> holds the work. One sequential network pump reads the game (fastest for both SMB shares and NVMe)
    /// while inflate and file writes fan out across cores, so on a cold install the wall clock is dominated by streaming the archive
    /// bytes; on warm re-runs the pump has nothing to do and the CPU lanes start at once.
    /// </summary>
    internal sealed class InstallPipeline
    {
        /// <summary>The archive extraction and the already-staged files both feed the NIF conversion.</summary>
        private const int MeshWorkProducers = 2;

        private readonly InstallLocations _paths;
        private readonly InstallOptions _options;
        private readonly MemoryBudget _memory;
        private readonly CancellationToken _ct;
        private readonly PhaseLog _phases = new();
        private readonly PipelineStages _stages;
        private readonly LaneRunner _lanes;
        private readonly PriorityFiles _skeletons;
        private readonly NifWorkQueue _meshWork;

        private readonly Gate _meshInputsReady = new("materials and actor hkx extracted");
        private readonly Gate _meshesExtracted = new("meshes extracted");
        private readonly Gate _textureIndexReady = new("texture index ready");
        private readonly Gate _esmCopied = new("ESM copied");
        private readonly Gate _texturesExtracted = new("textures extracted");
        private readonly Gate _skeletonGlbsReady = new("render skeleton GLBs converted");

        public InstallPipeline(InstallLocations paths, InstallOptions options, MemoryBudget memory, CancellationToken ct)
        {
            _paths = paths;
            _options = options;
            _memory = memory;
            _ct = ct;
            _stages = new PipelineStages(paths, memory, ct);
            _lanes = new LaneRunner(_phases, ct);
            _skeletons = new PriorityFiles(_skeletonGlbsReady.Open);
            _meshWork = new NifWorkQueue(MeshWorkProducers, _skeletons);
        }

        /// <summary>
        /// Runs every lane to the end, removes the staging folder unless <see cref="InstallOptions.KeepStaging"/> says to keep it, and prints
        /// the summary; throws the exception of the lane that failed or was cancelled, in which case the staging folder stays.
        /// </summary>
        public async Task RunAsync()
        {
            _paths.CreateOutputDirectories();
            Console.WriteLine(_memory.Describe());
            await InstallGameAsync();
            ConfigureConverters();
            using var lateBakes = new DeferredLateBakes();
            await CompleteAsync(StartWorkLanes(lateBakes));
        }

        /// <summary>Waits for the lanes that do the install, then removes the staging folder if that is wanted and only if they all succeeded.</summary>
        internal async Task CompleteAsync(IReadOnlyList<Task> workLanes)
        {
            var lanes = new List<Task>(workLanes);
            if (!_options.KeepStaging)
                lanes.Add(_lanes.Run(InstallSteps.CleanupLane, _stages.RemoveStaging, "remove temporary files", after: workLanes));
            await Task.WhenAll(lanes);

            _phases.Stop();
            _ct.ThrowIfCancellationRequested();
            _paths.MarkAssetsComplete(_options.EffectiveAssetVersion);
            InstallSummary.Print(_phases, InstallProgress.Model!.GetSnapshot(), _paths);
            Console.WriteLine();
            Console.WriteLine($"Assets deployed to: {_paths.AssetsRoot}");
        }

        /// <summary>Runs before every other lane: copying the game is quick, and a game that is still running (its files locked) is found before anything else is read or converted.</summary>
        internal Task InstallGameAsync() =>
            _lanes.Run(InstallSteps.GameLane, () => _stages.InstallGame(_options.Game ?? GameFiles.None), "install the game");

        private List<Task> StartWorkLanes(DeferredLateBakes lateBakes)
        {
            var pump = _lanes.Run(InstallSteps.PumpLane, RunPump,
                opens: [_textureIndexReady, _esmCopied, _meshInputsReady, _meshesExtracted, _texturesExtracted]);

            // Game archives only (Shaders.ba2, Interface.ba2) and their own output folders; no later stage reads the shaders, fonts,
            // UI layout or Pip-Boy art. They wait for the meshes so their archive reads never delay the extraction every CPU lane needs.
            var shaders = _lanes.Run(InstallSteps.ShaderLane, _stages.TranslateShaders, "translate shaders", after: [_meshesExtracted]);
            var interfaceAssets = _lanes.Run(InstallSteps.InterfaceLane, _stages.ExtractInterfaceAssets, "UI fonts + UI layout + Pip-Boy art",
                after: [_meshesExtracted]);

            // Game archives only (Sounds.ba2, Startup.ba2) into their own output folder; below-normal ffmpeg processes soak up what the
            // CPU lanes leave, and the archive reads wait for the meshes like the other game-archive lanes.
            var audio = _lanes.Run(InstallSteps.AudioLane, _stages.ConvertAudio, "audio -> FLAC", after: [_meshesExtracted]);

            // Reads the loose .bk2 files of the game's Data\Video; the decoder and ffmpeg run at below-normal priority and soak up the CPU the
            // other lanes leave. It waits for the meshes like the other lanes that must not slow the extraction every CPU lane needs.
            var video = _lanes.Run(InstallSteps.VideoLane, _stages.ConvertVideo, "video -> MP4", after: [_meshesExtracted]);

            var meshes = _lanes.Run(InstallSteps.MeshLane, () => _stages.ConvertMeshes(_meshWork, _skeletons), "NIF -> glTF",
                after: [_meshInputsReady, _textureIndexReady], opens: [_skeletonGlbsReady]);

            // Reads only the staged NIFs and writes only *.col.json and the manifest, and soaks up the CPU the other lanes leave idle.
            // It starts at once and decodes each NIF with collision while the mesh archives are still extracting: the work is latency
            // bound and runs at below-normal priority, so it takes only the CPU the extraction leaves. The staged NIFs the extraction
            // did not see written are queued after it.
            var collision = _lanes.Run(InstallSteps.CollisionLane, _stages.ExtractCollision, "collision sidecars");
            var collisionBacklog = _lanes.Run([], _stages.QueueUnobservedCollisionCandidates, "collision candidates left by earlier runs",
                after: [_meshesExtracted]);

            // Reads staged .hkx files and needs only the render skeleton GLBs of the NIF conversion, which come first there and only
            // after the GLB purge that would delete the clip packs; the rest of the NIF conversion runs beside it.
            var animations = _lanes.Run(InstallSteps.AnimationLane, _stages.ConvertAnimations, "Havok animations -> glTF",
                after: [_skeletonGlbsReady]);

            var palettes = _lanes.Run(InstallSteps.PaletteLane, _stages.BakePalettes, "palette bakes",
                after: [_meshesExtracted, _textureIndexReady, _texturesExtracted]);

            // Reads the ESM copy, the staged materials (census and texture-set material paths) and the texture index (the census validates
            // palette LUTs against it, so the index must exist before any census build); no staged mesh and no extracted texture. It
            // finishes right after its own work: the MODC swap synthesis, which reads the converted GLBs and the palette-baked textures,
            // is part of the tail.
            var database = _lanes.Run(InstallSteps.DatabaseLane, () => _stages.LoadDatabase(), "ESM -> SQLite",
                after: [_meshInputsReady, _esmCopied, _textureIndexReady]);

            // Game archives only; starts once the pump is done so its archive reads never compete with the pump.
            var lodWater = _lanes.Run(InstallSteps.LodWaterLane, _stages.ScanLodWater, "LOD water archive scan",
                after: [_texturesExtracted]);

            // Database: the tables the steps write to. GLBs: the models of the MODC swap synthesis and the LOD models of the backfill.
            // Palette bakes (which start after the texture extraction): the synthesis reads the baked textures and the late bakes read
            // the pristine originals they stashed.
            var tail = _lanes.Run(InstallSteps.TailLane, async () => _stages.FinalizeDatabase(lateBakes, database, await lodWater),
                "MODC swaps + late palette bakes + LOD water + LOD backfill + normalize", after: [database, meshes, palettes, lodWater]);

            return [pump, shaders, interfaceAssets, audio, video, meshes, collision, collisionBacklog, animations, database, palettes, lodWater, tail];
        }

        private void ConfigureConverters()
        {
            // The census decides palette-ness by LUT existence; textures aren't on disk when it builds, so route the check through
            // the texture index.
            MaterialCensus.TextureExistsOverride = NifConverter.TextureIndexContains;

            // Textures extract straight into assets\textures, so palette bakes read from there.
            PaletteBaker.SourceTexturesOverride = _paths.TexturesOut;
        }

        /// <summary>
        /// Reads the game in the order the other lanes need it: the texture index (name tables only), the ESM and INIs (the database lane
        /// needs them as soon as the materials are staged), the materials and actor .hkx files and the meshes, whose NIFs the conversion
        /// takes while they extract, then the textures and the colour LUTs.
        /// </summary>
        private void RunPump()
        {
            using var textures = _phases.Measure("texture index (BA2 name tables)", _stages.OpenAndIndexTextureArchives);
            _textureIndexReady.Open();

            _phases.Measure("copy ESM + INIs", () => _stages.CopyGameData(_esmCopied.Open));

            _phases.Measure("extract meshes+materials BA2s", () => _stages.ExtractMeshArchives(_meshWork, _skeletons, _meshInputsReady.Open));
            _meshesExtracted.Open();

            var extractStart = _phases.Elapsed;
            _stages.ExtractTextures(textures);
            _phases.Measure("extract colour LUTs (Misc.ba2)", _stages.ExtractColourLuts);
            _texturesExtracted.Open();
            if (textures.Pending.Count > 0) _phases.Record("extract texture BA2s", extractStart);
        }
    }
}
