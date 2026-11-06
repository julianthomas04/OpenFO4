using AssetConverter.Progress;

namespace AssetConverter.Pipeline
{
    /// <summary>
    /// The full install as concurrent lanes joined by gates (the diagram is in docs/installer/README.md, "Pipeline"). A lane starts
    /// the moment the gates and lanes it reads from are done; <see cref="RunAsync"/> declares every lane with its dependencies, and
    /// <see cref="PipelineStages"/> holds the work. One sequential network pump reads the game (fastest for both SMB shares and NVMe)
    /// while inflate and file writes fan out across cores, so on a cold install the wall clock is dominated by streaming the archive
    /// bytes; on warm re-runs the pump has nothing to do and the CPU lanes start at once. Which asset groups are converted at all is
    /// the <see cref="ReconversionPlan"/>'s business: a kept group's lanes complete at once and open the gates they would have opened.
    /// </summary>
    internal sealed class InstallPipeline
    {
        /// <summary>The archive extraction and the already-staged files both feed the NIF conversion.</summary>
        private const int MeshWorkProducers = 2;

        private readonly InstallLocations _paths;
        private readonly InstallOptions _options;
        private readonly ReconversionPlan _plan;
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
            _plan = options.EffectiveReconversion;
            _memory = memory;
            _ct = ct;
            _stages = new PipelineStages(paths, memory, ct);
            _lanes = new LaneRunner(_phases, ct);
            _skeletons = new PriorityFiles(_skeletonGlbsReady.Open);
            _meshWork = new NifWorkQueue(MeshWorkProducers, _skeletons);
        }

        /// <summary>
        /// Runs the lanes of every asset group the plan converts (the others are kept as they are) to the end, removes the staging folder
        /// unless <see cref="InstallOptions.KeepStaging"/> says to keep it, and prints the summary; throws the exception of the lane that
        /// failed or was cancelled, in which case the staging folder stays. The ownership marker lists only complete groups: it is cut
        /// down to the kept ones before any output is touched, and each converted group is added when its last lane has ended.
        /// </summary>
        public async Task RunAsync()
        {
            _paths.CreateOutputDirectories();
            Console.WriteLine(_memory.Describe());
            await InstallGameAsync();
            _paths.WithdrawReconverted(_plan, _options.EffectiveAssetVersion);
            ConfigureConverters();
            using var lateBakes = new DeferredLateBakes();
            await CompleteAsync(StartWorkLanes(lateBakes));
        }

        /// <summary>
        /// Waits for the lanes that do the install, then removes the staging folder if that is wanted and only if they all succeeded.
        /// The marker is not written here: each group recorded itself when its last lane ended, so it never lists a group that is unfinished.
        /// </summary>
        internal async Task CompleteAsync(IReadOnlyList<Task> workLanes)
        {
            var lanes = new List<Task>(workLanes);
            if (!_options.KeepStaging)
                lanes.Add(_lanes.Run(InstallSteps.CleanupLane, _stages.RemoveStaging, "remove temporary files", after: workLanes));
            await Task.WhenAll(lanes);

            _phases.Stop();
            _ct.ThrowIfCancellationRequested();
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
            var shaders = RunGroup(AssetGroup.Shaders, InstallSteps.ShaderLane, _stages.TranslateShaders, "translate shaders", after: [_meshesExtracted]);
            var interfaceAssets = RunGroup(AssetGroup.Interface, InstallSteps.InterfaceLane, _stages.ExtractInterfaceAssets, "UI fonts + UI layout + Pip-Boy art",
                after: [_meshesExtracted]);

            // Game archives only (Sounds.ba2, Startup.ba2) into their own output folder; below-normal ffmpeg processes soak up what the
            // CPU lanes leave, and the archive reads wait for the meshes like the other game-archive lanes.
            var audio = RunGroup(AssetGroup.Audio, InstallSteps.AudioLane, _stages.ConvertAudio, "audio -> FLAC", after: [_meshesExtracted]);

            // Reads the loose .bk2 files of the game's Data\Video; the decoder and ffmpeg run at below-normal priority and soak up the CPU the
            // other lanes leave. It waits for the meshes like the other lanes that must not slow the extraction every CPU lane needs.
            var video = RunGroup(AssetGroup.Video, InstallSteps.VideoLane, _stages.ConvertVideo, "video -> MP4", after: [_meshesExtracted]);

            var meshes = RunGroup(AssetGroup.Meshes, InstallSteps.MeshLane, () => _stages.ConvertMeshes(_meshWork, _skeletons), "NIF -> glTF",
                after: [_meshInputsReady, _textureIndexReady], opens: [_skeletonGlbsReady]);

            var (collision, collisionBacklog) = StartCollisionLanes(_lanes, _plan.Reconverts(AssetGroup.Collision), _meshesExtracted,
                _stages.QueueUnobservedCollisionCandidates, () => ConvertGroup(AssetGroup.Collision, _stages.ExtractCollision));

            // Reads staged .hkx files and needs only the render skeleton GLBs of the NIF conversion, which come first there and only
            // after the GLB purge that would delete the clip packs; the rest of the NIF conversion runs beside it. With the meshes kept,
            // their lane opens the gate as soon as the .hkx files are staged, since the skeletons are on disk already.
            var animations = RunGroup(AssetGroup.Animations, InstallSteps.AnimationLane, _stages.ConvertAnimations, "Havok animations -> glTF",
                after: [_skeletonGlbsReady]);

            // The textures and the database are one group: palette bakes overwrite the extracted textures in place and the tail bakes and
            // reads them again, so neither can be converted without the other. The group is complete when the tail has ended.
            bool texturesDatabase = _plan.Reconverts(AssetGroup.TexturesDatabase);

            var palettes = RunGroup(_lanes, texturesDatabase, InstallSteps.PaletteLane, _stages.BakePalettes, "palette bakes",
                after: [_meshesExtracted, _textureIndexReady, _texturesExtracted]);

            // Reads the ESM copy, the staged materials (census and texture-set material paths) and the texture index (the census validates
            // palette LUTs against it, so the index must exist before any census build); no staged mesh and no extracted texture. It
            // finishes right after its own work: the MODC swap synthesis, which reads the converted GLBs and the palette-baked textures,
            // is part of the tail.
            var database = RunGroup(_lanes, texturesDatabase, InstallSteps.DatabaseLane, () => _stages.LoadDatabase(), "ESM -> SQLite",
                after: [_meshInputsReady, _esmCopied, _textureIndexReady]);

            // Game archives only; starts once the pump is done so its archive reads never compete with the pump.
            LodWaterHeights? lodWaterHeights = null;
            var lodWater = RunGroup(_lanes, texturesDatabase, InstallSteps.LodWaterLane, () => lodWaterHeights = _stages.ScanLodWater(), "LOD water archive scan",
                after: [_texturesExtracted]);

            // Database: the tables the steps write to. GLBs: the models of the MODC swap synthesis and the LOD models of the backfill.
            // Palette bakes (which start after the texture extraction): the synthesis reads the baked textures and the late bakes read
            // the pristine originals they stashed.
            var tail = RunGroup(_lanes, texturesDatabase, InstallSteps.TailLane,
                () => MarkGroupComplete(AssetGroup.TexturesDatabase, () => _stages.FinalizeDatabase(lateBakes, database,
                    lodWaterHeights ?? throw new InvalidOperationException("The LOD water scan finished without a result."))),
                "MODC swaps + late palette bakes + LOD water + LOD backfill + normalize", after: [database, meshes, palettes, lodWater]);

            return [pump, shaders, interfaceAssets, audio, video, meshes, collision, collisionBacklog, animations, database, palettes, lodWater, tail];
        }

        /// <summary>The single lane of <paramref name="group"/>: when the plan converts the group, its output is purged first and the marker lists it once the work has returned.</summary>
        private Task RunGroup(AssetGroup group, IReadOnlyList<string> steps, Action work, string? phase = null,
            IReadOnlyList<Task>? after = null, IReadOnlyList<Gate>? opens = null) =>
            RunGroup(_lanes, _plan.Reconverts(group), steps, () => ConvertGroup(group, work), phase, after, opens);

        private void ConvertGroup(AssetGroup group, Action work) =>
            Converting(group, _paths, _options.EffectiveAssetVersion, work, _ct)();

        private void MarkGroupComplete(AssetGroup group, Action work) =>
            Recording(group, _paths, _options.EffectiveAssetVersion, work)();

        /// <summary>The conversion of <paramref name="group"/>: its output is purged first, which also defeats the conversion stamps inside it, then <paramref name="work"/> converts it and the marker lists it.</summary>
        internal static Action Converting(AssetGroup group, InstallLocations paths, string installerVersion, Action work, CancellationToken ct = default) =>
            Recording(group, paths, installerVersion, () =>
            {
                GroupPurge.Purge(group, paths, ct: ct);
                work();
            });

        /// <summary>The marker lists <paramref name="group"/> once <paramref name="work"/> has returned and not before, so it never vouches for output that is unfinished.</summary>
        internal static Action Recording(AssetGroup group, InstallLocations paths, string installerVersion, Action work) => () =>
        {
            work();
            paths.MarkGroupComplete(group, installerVersion);
        };

        /// <summary>
        /// A lane of an asset group. A group the plan keeps has nothing to convert: its lane waits for what the real one would wait for,
        /// then completes after opening every gate the real one would have opened, so each lane behind it starts as it would have and none
        /// waits for ever. It owns no progress step, since a step that is skipped must never appear in the progress.
        /// </summary>
        internal static Task RunGroup(LaneRunner lanes, bool reconverts, IReadOnlyList<string> steps, Action work, string? phase = null,
            IReadOnlyList<Task>? after = null, IReadOnlyList<Gate>? opens = null) =>
            reconverts
                ? lanes.Run(steps, work, phase, after, opens)
                : lanes.Run([], () => OpenAll(opens), after: after, opens: opens);

        internal static Task RunGroup(LaneRunner lanes, bool reconverts, IReadOnlyList<string> steps, Func<Task> work, string? phase = null,
            IReadOnlyList<Task>? after = null, IReadOnlyList<Gate>? opens = null) =>
            reconverts
                ? lanes.Run(steps, work, phase, after, opens)
                : lanes.Run([], () => OpenAll(opens), after: after, opens: opens);

        private static void OpenAll(IReadOnlyList<Gate>? gates)
        {
            foreach (var gate in gates ?? [])
                gate.Open();
        }

        /// <summary>
        /// Collision reads only the staged NIFs and writes only *.col.json and the manifest, and soaks up the CPU the other lanes leave
        /// idle: the work is latency bound and runs at below-normal priority. It waits for the mesh extraction and for the staged NIFs the
        /// extraction did not see written to be queued, so the set of NIFs it decodes, which is the total of its progress, is final when
        /// it starts. A failure of the queuing fails the collision lane without it ever starting. A kept group runs neither lane.
        /// </summary>
        internal static (Task Collision, Task Backlog) StartCollisionLanes(LaneRunner lanes, bool reconverts, Task meshesExtracted, Action queueUnobservedCandidates, Action extractCollision)
        {
            var backlog = RunGroup(lanes, reconverts, [], queueUnobservedCandidates, "collision candidates left by earlier runs", after: [meshesExtracted]);
            var collision = RunGroup(lanes, reconverts, InstallSteps.CollisionLane, extractCollision, "collision sidecars", after: [backlog]);
            return (collision, backlog);
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
        /// takes while they extract, then the textures and the colour LUTs. Only what the converted groups need is read (see
        /// <see cref="PumpWork"/>); the gates of the steps left out open all the same.
        /// </summary>
        private void RunPump()
        {
            var work = PumpWork.For(_plan.Reconverted);

            // The textures extract straight from their archives, whose done-markers the index reads: purge before it.
            if (_plan.Reconverts(AssetGroup.TexturesDatabase))
                GroupPurge.Purge(AssetGroup.TexturesDatabase, _paths, ct: _ct);

            using var textures = IndexTextures(work);
            _textureIndexReady.Open();

            if (work.CopyGameData)
                _phases.Measure("copy ESM + INIs", () => _stages.CopyGameData(_esmCopied.Open));
            _esmCopied.Open();

            if (work.ExtractMeshArchives)
                _phases.Measure("extract meshes+materials BA2s", () => _stages.ExtractMeshArchives(_meshWork, _skeletons, _meshInputsReady.Open));
            else
                _meshInputsReady.Open();
            _meshesExtracted.Open();

            if (textures is not null)
                ExtractTextures(textures);
            _texturesExtracted.Open();
        }

        /// <returns>The texture archives to extract, opened, with the index seeded from them; null when the textures are kept, in which case at most the names are indexed.</returns>
        private TextureArchiveSet? IndexTextures(PumpWork work)
        {
            if (work.ExtractTextures)
                return _phases.Measure("texture index (BA2 name tables)", _stages.OpenAndIndexTextureArchives);
            if (work.IndexTextureNames)
                _phases.Measure("texture index (BA2 name tables)", _stages.IndexTextureNames);
            return null;
        }

        private void ExtractTextures(TextureArchiveSet textures)
        {
            var extractStart = _phases.Elapsed;
            _stages.ExtractTextures(textures);
            _phases.Measure("extract colour LUTs (Misc.ba2)", _stages.ExtractColourLuts);
            if (textures.Pending.Count > 0) _phases.Record("extract texture BA2s", extractStart);
        }
    }
}
