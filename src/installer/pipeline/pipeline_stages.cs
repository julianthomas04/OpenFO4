using AssetConverter.Audio;
using AssetConverter.Progress;
using AssetConverter.ShaderTranslation.Installation;
using AssetConverter.UiLayout;
using AssetConverter.Video;

namespace AssetConverter.Pipeline
{
    /// <summary>
    /// The work of each stage of the full install, one method per stage. When and after what each runs is <see cref="InstallPipeline"/>'s
    /// business; a stage reports its own progress phase and checks for cancellation between its sub-steps.
    /// </summary>
    internal sealed class PipelineStages : IPipelineStages
    {
        private const string MeshesArchiveName = "Fallout4 - Meshes.ba2";
        private const string MeshesExtraArchiveName = "Fallout4 - MeshesExtra.ba2";
        private const string MaterialsArchiveName = "Fallout4 - Materials.ba2";
        private const string StagedMeshesPrefix = "Meshes\\";
        private const string StagedMaterialsPrefix = "Materials\\";
        private const string NifExtension = ".nif";
        private static readonly string[] MaterialExtensions = [".bgsm", ".bgem"];

        private static readonly string[] TextureArchives =
        [
            "Fallout4 - Textures1.ba2", "Fallout4 - Textures2.ba2", "Fallout4 - Textures3.ba2", "Fallout4 - Textures4.ba2",
            "Fallout4 - Textures5.ba2", "Fallout4 - Textures6.ba2", "Fallout4 - Textures7.ba2", "Fallout4 - Textures8.ba2",
            "Fallout4 - Textures9.ba2", "Fallout4 - TexturesPatch.ba2",
        ];

        /// <summary>The archive extraction and the staged NIFs it did not see both feed the collision worker.</summary>
        internal const int CollisionWorkProducers = 2;

        private readonly InstallLocations _paths;
        private readonly MemoryBudget _memory;
        private readonly CancellationToken _ct;
        private readonly CollisionCandidates _collisionCandidates = new();
        private readonly Lazy<string> _language;
        private Func<Task, int>? _drainLateBakes;
        private LodWaterHeights? _lodWater;

        public PipelineStages(InstallLocations paths, MemoryBudget memory, CancellationToken ct)
        {
            _paths = paths;
            _memory = memory;
            _ct = ct;
            _language = new Lazy<string>(() => GameLanguage.Resolve(paths.Fo4Location, paths.SettingsDirectory));
        }

        /// <summary>The installed language (<c>sLanguage</c> of the game's INIs) the localized files are taken in.</summary>
        private string Language => _language.Value;

        /// <summary>The NIFs the collision worker decodes: <see cref="ExtractMeshArchives"/> queues the ones it saw carry the collision marker, <see cref="QueueUnobservedCollisionCandidates"/> the rest it has no verdict for.</summary>
        internal NifWorkQueue CollisionWork { get; } = new(CollisionWorkProducers);

        /// <summary>
        /// Materials, the actor .hkx files, then the mesh archives: what the CPU stages need first, in the order they need it. Each NIF is
        /// handed to <paramref name="meshes"/> the moment it is complete, so the conversion runs while the archives are still extracting.
        /// A failure ends the queue with the same exception, so the workers waiting on it never hang. Each NIF that carries the collision
        /// marker is recorded in <see cref="CollisionWork"/> in the same way, which the collision step reads once the extraction is over.
        /// </summary>
        /// <param name="skeletons">Told which staged NIFs are the render skeletons, as soon as the .hkx files they derive from are staged.</param>
        /// <param name="onInputsReady">Called once the materials and the .hkx files are staged: all the NIF conversion needs besides the NIFs themselves. Material files in the mesh archives
        /// count, so the call comes after the last archive that holds one, which is later than the .hkx files when the mesh archives do.</param>
        public bool ExtractMeshArchives(NifWorkQueue meshes, PriorityFiles skeletons, Action onInputsReady)
        {
            try
            {
                using var extraction = StagingExtraction.Open(_paths.GameDataDirectory, _paths.StagingRoot, MeshSources(meshes, skeletons), _memory);
                extraction.WhenWritten(IsMaterialInput, HavokAnimationStage.ArchiveName, onInputsReady);
                meshes.Announce(extraction.WrittenFiles.Where(IsStagedNif).Select(StagedPath));

                int phase = InstallProgress.BeginPhase(InstallSteps.ExtractMeshes, extraction.WrittenFiles.Count);
                bool allSucceeded = extraction.Run(phase, _ct);
                InstallProgress.EndPhase(phase);
                meshes.Complete();
                CollisionWork.Complete();
                return allSucceeded;
            }
            catch (Exception ex)
            {
                meshes.Fail(ex);
                CollisionWork.Fail(ex);
                throw;
            }
        }

        /// <summary>
        /// For dev commands that read the staged NIFs but run without a staging folder: extracts the mesh archives the full install extracts,
        /// by the same sources and markers, so archives already marked complete are skipped.
        /// </summary>
        /// <exception cref="FileNotFoundException">A mesh archive is missing from the game's Data folder.</exception>
        /// <exception cref="InvalidOperationException">An entry failed to extract, or the extraction left no staged meshes behind.</exception>
        public void EnsureStagedMeshArchives()
        {
            foreach (string name in new[] { MeshesArchiveName, MeshesExtraArchiveName })
            {
                string archivePath = Path.Combine(_paths.GameDataDirectory, name);
                if (!File.Exists(archivePath)) throw new FileNotFoundException($"{archivePath} not found; the meshes cannot be staged.", archivePath);
            }

            using var extraction = StagingExtraction.Open(_paths.GameDataDirectory, _paths.StagingRoot, [ArchiveSource(MeshesArchiveName), ArchiveSource(MeshesExtraArchiveName)], _memory);
            if (extraction.WrittenFiles.Count > 0)
            {
                Console.WriteLine($"The staged meshes are missing or incomplete: extracting {extraction.WrittenFiles.Count:N0} files from the mesh archives into {_paths.StagingRoot} first. This is slow.");
                Directory.CreateDirectory(_paths.StagingRoot);
                if (!extraction.Run(-1, _ct))
                    throw new InvalidOperationException("Some mesh archive entries failed to extract; the staged meshes are incomplete.");
            }
            if (!Directory.Exists(_paths.StagedMeshes))
                throw new InvalidOperationException($"The extraction markers claim the mesh archives are staged, but {_paths.StagedMeshes} does not exist; delete the markers (.Fallout4 - Meshes*.extracted) and run again.");
        }

        /// <summary>
        /// For dev commands that read the staged materials but run without a staging folder: extracts the materials archive by the same
        /// source and marker as the full install, so an archive already marked complete is skipped.
        /// </summary>
        /// <exception cref="FileNotFoundException">The materials archive is missing from the game's Data folder.</exception>
        /// <exception cref="InvalidOperationException">An entry failed to extract, or the extraction left no staged materials behind.</exception>
        public void EnsureStagedMaterials()
        {
            string archivePath = Path.Combine(_paths.GameDataDirectory, MaterialsArchiveName);
            if (!File.Exists(archivePath)) throw new FileNotFoundException($"{archivePath} not found; the materials cannot be staged.", archivePath);

            using var extraction = StagingExtraction.Open(_paths.GameDataDirectory, _paths.StagingRoot, [ArchiveSource(MaterialsArchiveName)], _memory);
            if (extraction.WrittenFiles.Count > 0)
            {
                Console.WriteLine($"The staged materials are missing or incomplete: extracting {extraction.WrittenFiles.Count:N0} files from the materials archive into {_paths.StagingRoot} first.");
                Directory.CreateDirectory(_paths.StagingRoot);
                if (!extraction.Run(-1, _ct))
                    throw new InvalidOperationException("Some materials archive entries failed to extract; the staged materials are incomplete.");
            }
            if (!Directory.Exists(_paths.StagedMaterials))
                throw new InvalidOperationException($"The extraction marker claims the materials archive is staged, but {_paths.StagedMaterials} does not exist; delete the marker (.{MaterialsArchiveName}.extracted) and run again.");
        }

        private IReadOnlyList<StagingSource> MeshSources(NifWorkQueue meshes, PriorityFiles skeletons)
        {
            var onDisk = new StagedPathSpelling(_paths.StagingRoot);

            StagingSource NifArchiveSource(string archiveName) => ArchiveSource(archiveName) with
            {
                Watches = IsStagedNif,
                OnFileWritten = stagedPath => HandOver(meshes, stagedPath, onDisk.Resolve(stagedPath)),
                ObserveContent = stagedPath => _collisionCandidates.Observe(StagedPath(stagedPath)),
            };

            return
            [
                ArchiveSource(MaterialsArchiveName),
                HavokAnimationStage.ActorHkxSource with
                {
                    OnFinished = () => skeletons.Prioritize(HavokAnimationStage.RenderSkeletonSources(_paths.StagingRoot, _paths.MeshesOut)),
                },
                NifArchiveSource(MeshesArchiveName),
                NifArchiveSource(MeshesExtraArchiveName),
            ];
        }

        /// <summary>A written NIF is complete on disk, and its collision verdict is in, because the content observer completes before the write callback runs.</summary>
        private void HandOver(NifWorkQueue meshes, string stagedPath, string onDiskPath)
        {
            meshes.Add(onDiskPath);
            if (_collisionCandidates.HasMarker(StagedPath(stagedPath)))
                CollisionWork.Add(onDiskPath);
        }

        private static StagingSource ArchiveSource(string archiveName) => new(archiveName, $".{archiveName}.extracted", _ => true);

        private string StagedPath(string stagingRelativePath) => Path.Combine(_paths.StagingRoot, ArchivePath.Normalize(stagingRelativePath));

        private static bool IsStagedNif(string stagingRelativePath) => ArchivePath.Matches(stagingRelativePath, StagedMeshesPrefix, NifExtension);

        /// <summary>The staged files the NIF conversion and the ESM load read the materials from: the census scans the whole staging tree for material files.</summary>
        private static bool IsMaterialInput(string stagingRelativePath) =>
            ArchivePath.Matches(stagingRelativePath, StagedMaterialsPrefix, "")
            || MaterialExtensions.Any(extension => stagingRelativePath.EndsWith(extension, StringComparison.OrdinalIgnoreCase));

        /// <summary>
        /// The census decides palette-ness by LUT existence and the textures are not on disk when it builds, so its check goes through the
        /// texture index; the palette bakes read the textures from where they extract straight into; and the database load queues the late
        /// bakes it finds instead of baking them. Disposing the result ends the deferral.
        /// </summary>
        public IDisposable ConfigureConverters()
        {
            MaterialCensus.TextureExistsOverride = NifConverter.TextureIndexContains;
            PaletteBaker.SourceTexturesOverride = _paths.TexturesOut;
            var lateBakes = new DeferredLateBakes();
            _drainLateBakes = databaseLoaded => lateBakes.Drain(databaseLoaded, _paths.StagingRoot, _paths.TexturesOut);
            return lateBakes;
        }

        /// <summary>
        /// Opens the texture archives that still need extracting and seeds the texture-existence index from their name tables, so NIF
        /// conversion can start before a single texture byte is extracted. The caller owns and disposes the set.
        /// </summary>
        public TextureArchiveSet OpenAndIndexTextureArchives()
        {
            var textures = TextureArchiveSet.Open(_paths.GameDataDirectory, _paths.StagingRoot, TextureArchives);
            try
            {
                NifConverter.PreloadTextureIndex(textures.IndexNames(_paths.TexturesOut));
            }
            catch
            {
                textures.Dispose();
                throw;
            }
            return textures;
        }

        /// <summary>
        /// One local ESM copy (the load reads the file three times: Mutagen overlay + two raw scans, never pay network latency three
        /// times), then the string tables the localized ESM resolves its names from.
        /// </summary>
        /// <param name="onEsmCopied">Called as soon as the local ESM is complete, while the string tables are still extracted.</param>
        public void CopyGameData(Action onEsmCopied)
        {
            int phase = InstallProgress.BeginPhase(InstallSteps.CopyGameData, InstallSteps.CopyGameDataTotal);
            InstallProgress.SetDetail(phase, "Fallout4.esm");
            CopyEsm();
            _ct.ThrowIfCancellationRequested();
            InstallProgress.Tick(phase);

            RunGameDataStep(phase, "string tables", () => StringTables.Extract(_paths.Fo4Location, _paths.StagingRoot, Language));
            onEsmCopied();
            InstallProgress.EndPhase(phase);
        }

        /// <summary>
        /// The INI cascade of the game into the install. Every install does this, whichever asset groups it converts, so it belongs to no progress
        /// step: it is quick, and an INI edited since the last install must be picked up.
        /// </summary>
        public void CopyGameInis()
        {
            _ct.ThrowIfCancellationRequested();
            InstallOperations.CopyGameInis(_paths.Fo4Location, _paths.DataOut, _paths.SettingsDirectory);
        }

        /// <summary>
        /// FO4's own UI fonts, Pip-Boy art and the UI layout documents. Each extractor first checks whether its output is current (the fonts
        /// and the art may start an ffdec JVM to read its version); the step's total is the number of extractors that have work, so one
        /// with everything cached shows as skipped, and the pending ones then run, each ticking once.
        /// </summary>
        public void ExtractInterfaceAssets() =>
            ExtractInterfaceAssets(
            [
                () => FontExtractor.Plan(_paths.Fo4Location, _paths.SettingsDirectory, _paths.StagingRoot, _paths.FontsOut, _memory, _ct),
                () => UiLayoutExtractor.Plan(_paths.Fo4Location, _paths.SettingsDirectory, _paths.UiLayoutOut),
                () => PipboyArtExtractor.Plan(_paths.Fo4Location, _paths.StagingRoot, _paths.PipboyArtOut, _memory, _ct),
            ]);

        internal void ExtractInterfaceAssets(IReadOnlyList<Func<PendingExtraction?>> plans)
        {
            int phase = InstallProgress.BeginPhase(InstallSteps.ExtractInterface, null);
            InstallProgress.SetDetail(phase, "checking which assets are out of date");
            var pending = new List<PendingExtraction>();
            foreach (var plan in plans)
            {
                _ct.ThrowIfCancellationRequested();
                if (plan() is { } extraction) pending.Add(extraction);
            }
            InstallProgress.AddTotal(phase, pending.Count);
            InstallProgress.SealTotal(phase);
            foreach (var extraction in pending)
            {
                _ct.ThrowIfCancellationRequested();
                InstallProgress.SetDetail(phase, extraction.Detail);
                extraction.Run();
                _ct.ThrowIfCancellationRequested();
                InstallProgress.Tick(phase);
            }
            InstallProgress.EndPhase(phase);
        }

        /// <summary>FO4's shaders: Shaders.ba2 -> DXBC -> WGSL the engine composes with its own wrappers.</summary>
        public void TranslateShaders()
        {
            int phase = InstallProgress.BeginPhase(InstallSteps.ConvertShaders, null);
            ShaderTranslateStep.Run(_paths.Fo4Location, _paths.AssetsRoot, InstallerPaths.Require(ShaderManifestReader.RepoRelativePath), phase, _ct);
            _ct.ThrowIfCancellationRequested();
            InstallProgress.EndPhase(phase);
        }

        /// <summary>FO4's sound and music: Sounds.ba2 and Startup.ba2 -> FLAC through ffmpeg, straight into assets\audio.</summary>
        public void ConvertAudio() => ConvertAudio(force: false);

        /// <param name="force">Converts everything again even where the recorded conversion is current.</param>
        public void ConvertAudio(bool force)
        {
            var ffmpeg = Ffmpeg.Installed();
            string encoder = $"{ffmpeg.Version()} arguments:{FlacEncoding.SettingsFingerprint}";
            using var conversion = AudioConversion.Plan(_paths.GameDataDirectory, _paths.AudioOut, FlacEncoding.With(ffmpeg), encoder, force: force, memory: _memory);
            int phase = InstallProgress.BeginPhase(InstallSteps.ConvertAudio, conversion.PendingFileCount);
            conversion.Run(phase, _ct);
            InstallProgress.EndPhase(phase);
        }

        /// <summary>FO4's videos: every Data\Video\*.bk2 -> Bink2 decoder -> ffmpeg (H.264 + AAC) -> MP4, straight into assets\video.</summary>
        public void ConvertVideo() => ConvertVideo(force: false);

        /// <param name="force">Converts everything again even where the recorded conversion is current.</param>
        public void ConvertVideo(bool force) => ConvertVideo(force, Ffmpeg.Installed(), Bink2Decoder.Installed());

        internal void ConvertVideo(bool force, Ffmpeg ffmpeg, Bink2Decoder decoder)
        {
            string encoder = $"{ffmpeg.Version()} decoder:{decoder.Version()} arguments:{VideoEncoding.SettingsFingerprint}";
            var conversion = VideoConversion.Plan(_paths.GameVideoDirectory, _paths.VideoOut, VideoTranscoding.With(decoder, ffmpeg), encoder, force: force, memory: _memory);
            int phase = InstallProgress.BeginPhase(InstallSteps.ConvertVideo, conversion.PendingCount);
            conversion.Run(phase, _ct);
            InstallProgress.EndPhase(phase);
        }

        /// <summary>
        /// Texture archives extract straight into assets\textures with the "textures\" prefix stripped; there is no staging copy.
        /// <see cref="TextureExtraction.ConcurrentArchives"/> archives are read at once, after the mesh archives, which must not share the source.
        /// </summary>
        public bool ExtractTextures(TextureArchiveSet textures)
        {
            var extraction = new TextureExtraction(textures, memory: _memory);
            int phase = InstallProgress.BeginPhase(InstallSteps.ExtractTextures, extraction.FileCount);
            bool allSucceeded = extraction.Run(_paths.TexturesOut, phase, BA2ParallelUnpacker.DefaultMaxInFlightBytes, _ct);
            InstallProgress.EndPhase(phase);
            if (!allSucceeded)
                Console.Error.WriteLine("WARNING: some texture archives had extraction failures; markers withheld, re-run to repair.");
            return allSucceeded;
        }

        /// <summary>The image spaces' colour LUTs, from Misc.ba2 straight into assets	extures like every other texture.</summary>
        /// <exception cref="FileNotFoundException">Misc.ba2 is missing from the game's Data folder.</exception>
        /// <exception cref="InvalidOperationException">A LUT failed to extract, or the archive holds none.</exception>
        public void ExtractColourLuts()
        {
            string archivePath = Path.Combine(_paths.GameDataDirectory, ColourLuts.ArchiveName);
            if (!File.Exists(archivePath))
                throw new FileNotFoundException($"{archivePath} not found; the image spaces' colour LUTs cannot be extracted.", archivePath);
            var result = BA2ParallelUnpacker.Unpack(archivePath, _paths.TexturesOut, _memory, quiet: true,
                stripFirstSegment: TextureExtraction.StrippedSegment, entryFilter: ColourLuts.IsColourLut, ct: _ct);
            if (result.Total == 0)
                throw new InvalidOperationException($"{archivePath} holds no colour LUT below {ColourLuts.ArchiveFolder}.");
            if (!result.AllSucceeded)
                throw new InvalidOperationException($"{result.Failed} of {result.Total} colour LUTs failed to extract from {archivePath}.");
        }

        /// <summary>
        /// Purges the previous run's GLBs, then converts every NIF of <paramref name="meshes"/> as it arrives, together with those already
        /// staged by an earlier run; the render skeletons of the Havok units go first.
        /// </summary>
        /// <param name="meshes">Fed with the NIFs of the archives still being extracted; this stage is its second producer.</param>
        /// <param name="skeletons">Signals once every render skeleton GLB is written or failed. Always after the purge, so the clip packs the Havok lane then writes next to the skeletons are never purged.</param>
        public void ConvertMeshes(NifWorkQueue meshes, PriorityFiles skeletons)
        {
            var staged = Task.Run(EnumerateStagedNifs, _ct);
            InstallOperations.PurgeByExtension(_paths.MeshesOut, "*.glb", "GLB meshes", ct: _ct);
            meshes.AddExisting(staged.GetAwaiter().GetResult());
            meshes.Complete();
            skeletons.Expect(meshes.Expected);

            int expected = meshes.ExpectedCount;
            int phase = InstallProgress.BeginPhase(InstallSteps.ConvertMeshes, expected);
            InstallOperations.UseEffectShaderCatalog(_paths.Fo4Location);
            int converted = NifConverter.ConvertToGltfBatched(_paths.StagedMeshes, _paths.MeshesOut, meshes, bgsmScanRoot: _paths.StagingRoot,
                progressPhase: phase, priorityFiles: skeletons, ct: _ct, memory: _memory);
            if (MeshCountMismatch(converted, expected) is { } mismatch)
                Console.Error.WriteLine(mismatch);
            _ct.ThrowIfCancellationRequested();
            InstallProgress.EndPhase(phase);
        }

        /// <summary>
        /// The total of the step is fixed before the conversion starts, so a conversion that handled another number of NIFs than were
        /// announced cannot adjust it; this is the warning for that, or null when the counts agree.
        /// </summary>
        internal static string? MeshCountMismatch(int processed, int announced) =>
            processed == announced
                ? null
                : $"WARNING: the mesh conversion handled {processed:N0} NIFs but {announced:N0} were announced; the step's progress count was based on the announced number.";

        private string[] EnumerateStagedNifs() =>
            Directory.Exists(_paths.StagedMeshes) ? NifConverter.EnumerateNifs(_paths.StagedMeshes) : [];

        /// <summary>
        /// Starts the collision worker once <see cref="CollisionWork"/> is final and feeds it the queued NIFs; it writes *.col.json next to
        /// the GLBs and ends once the queue is drained. The NIFs extracted by this run were checked for the worker's marker while they
        /// were written and queued the moment they were complete; <see cref="QueueUnobservedCollisionCandidates"/> queued the rest and
        /// finished the queue, so its size is the step's total.
        /// </summary>
        public void ExtractCollision()
        {
            int phase = InstallProgress.BeginPhase(InstallSteps.ExtractCollision, CollisionWork.ExpectedCount);
            CollisionSidecars.PurgeIfStale(_paths.MeshesOut, _ct);
            NifConverter.ExtractFo4Collision(_paths.StagedMeshes, _paths.MeshesOut, CollisionWork, phase, _memory, _ct);
            CollisionSidecars.RecordCurrent(_paths.MeshesOut);
            InstallProgress.EndPhase(phase);
        }

        /// <summary>
        /// Once the mesh archives are extracted, queues the staged NIFs this run did not see written (left by an earlier run, or whose
        /// extraction failed part-way) and not seen without the marker. The worker's own prefilter rejects those without collision, since
        /// checking them here would read every one of them again. A failure ends <see cref="CollisionWork"/> with the same exception.
        /// </summary>
        public void QueueUnobservedCollisionCandidates()
        {
            try
            {
                var staged = EnumerateStagedNifs();
                int unobserved = CollisionWork.AddExisting(_collisionCandidates.Select(staged));
                Console.WriteLine($"Collision candidates: {CollisionWork.ExpectedCount} of {staged.Length} staged NIFs ({unobserved} not seen during the extraction).");
                CollisionWork.Complete();
            }
            catch (Exception ex)
            {
                CollisionWork.Fail(ex);
                throw;
            }
        }

        /// <summary>Staged .hkx files and the render skeleton GLBs -> clip packs.</summary>
        public void ConvertAnimations() => HavokAnimationStage.Convert(_paths.StagingRoot, _paths.MeshesOut, _memory, _ct);

        /// <summary>
        /// The census (staging materials + texture index, built here if no other lane built it yet) and the extracted textures. Winner
        /// bakes replace deployed textures in place.
        /// </summary>
        public void BakePalettes()
        {
            int phase = InstallProgress.BeginPhase(InstallSteps.BakePalettes, InstallSteps.BakePalettesTotal);
            PaletteBaker.BakeAll(_paths.StagingRoot, _paths.TexturesOut, _ct);
            _ct.ThrowIfCancellationRequested();
            InstallProgress.Tick(phase);
            InstallProgress.EndPhase(phase);
        }

        public async Task LoadDatabase()
        {
            var loader = new ESMLoader(_paths.DatabasePath, materialsRoot: _paths.StagingRoot, texturesOut: _paths.TexturesOut)
            {
                Strings = new StringTableSet(StringTables.Directory(_paths.StagingRoot), Language),
                Memory = _memory,
            };
            int phase = InstallProgress.BeginPhase(InstallSteps.BuildDatabase, null);
            await loader.LoadESMAsync(_paths.StagedEsm, ct: _ct, progressPhase: phase).ConfigureAwait(false);
            InstallProgress.EndPhase(phase);
        }

        /// <summary>Reads only the game's archives; the database write waits for <see cref="FinalizeDatabase"/>.</summary>
        public void ScanLodWater()
        {
            int phase = InstallProgress.BeginPhase(InstallSteps.ScanLodWater, InstallSteps.ScanLodWaterTotal);
            var heights = LodWaterStage.Scan(_paths.GameDataDirectory, _ct);
            _ct.ThrowIfCancellationRequested();
            InstallProgress.Tick(phase);
            InstallProgress.EndPhase(phase);
            _lodWater = heights;
        }

        /// <summary>
        /// Everything that needs the finished database, the converted GLBs, the extracted textures and the palette bakes. The MODC swap
        /// synthesis comes first: it reads the model paths as the ESM load wrote them and the statics before the LOD backfill.
        /// </summary>
        public void FinalizeDatabase(Task databaseLoaded)
        {
            var drainLateBakes = _drainLateBakes ?? throw new InvalidOperationException("The converters were not configured before the database was finalized.");
            var lodWater = _lodWater ?? throw new InvalidOperationException("The LOD water was not scanned before the database was finalized.");
            (string Detail, Action Run)[] steps =
            [
                ("MODC material swaps", () => ModcSwapSynthesizer.Run(_paths.DatabasePath, _paths.MeshesOut, _paths.StagingRoot, _paths.TexturesOut, _ct)),
                ("late palette bakes", () => drainLateBakes(databaseLoaded)),
                ("LOD water", () => LodWaterStage.WriteTable(_paths.DatabasePath, lodWater)),
                ("LOD backfill", () => LodBackfill.Run(_paths.DatabasePath, _paths.MeshesOut)),
                ("door trigger bounds", () => DoorTriggerBounds.RunOnGame(_paths.DatabasePath, _paths.GameDataDirectory)),
                ("model path normalization", () => ModelPathNormalizer.Run(_paths.DatabasePath)),
            ];
            int phase = InstallProgress.BeginPhase(InstallSteps.FinalizeDatabase, steps.Length);
            foreach (var (detail, run) in steps) RunFinalizeStep(phase, detail, run);
            InstallProgress.EndPhase(phase);
        }

        /// <summary>
        /// Puts the game's own files into the install folder. A packaged installer always has them, so their absence is an error; an
        /// installer run from the repository before the engine was built has none, which is a warning and leaves the install without a game.
        /// </summary>
        /// <exception cref="FileNotFoundException">The game files are required and there is no game executable among them.</exception>
        public void InstallGame(GameFiles game)
        {
            int phase = InstallProgress.BeginPhase(InstallSteps.InstallGame, game.Files.Count);
            if (!game.HasExecutable)
            {
                if (game.Required)
                    throw new FileNotFoundException($"The installer carries no {GameFiles.ExecutableName}; the installer is damaged or was built without the game.");
                Console.Error.WriteLine($"WARNING: no {GameFiles.ExecutableName} to install (build the engine first); the install will have no game executable.");
                InstallProgress.SetDetail(phase, "no game executable built");
                InstallProgress.EndPhase(phase);
                return;
            }
            GameFileInstaller.Install(game, _paths.InstallRoot, name =>
            {
                Console.WriteLine($"Installed {name} into {_paths.InstallRoot}");
                InstallProgress.Tick(phase);
            }, _ct);
            InstallProgress.EndPhase(phase);
        }

        /// <summary>
        /// Deletes the staging folder once everything that reads it has finished. The converted output is complete by then, so a folder or
        /// file that cannot be removed is a warning, not a failure: the only cost is the disk space.
        /// </summary>
        public StagingRemovalResult RemoveStaging()
        {
            int phase = InstallProgress.BeginPhase(InstallSteps.RemoveStaging, null);
            var elapsed = System.Diagnostics.Stopwatch.StartNew();
            var result = StagingRemoval.Run(_paths.StagingRoot, phase, ct: _ct);
            Console.WriteLine($"Removed temporary files: {result.FilesRemoved:N0} files and {result.FoldersRemoved:N0} folders in {ProgressFormatter.Invariant.Duration(elapsed.Elapsed)}.");
            if (result.Failures.Count > 0) ReportRemovalFailures(phase, result);
            InstallProgress.EndPhase(phase);
            return result;
        }

        private void ReportRemovalFailures(int phase, StagingRemovalResult result)
        {
            const int Listed = 5;
            string shown = string.Join("; ", result.Failures.Take(Listed));
            string more = result.Failures.Count > Listed ? $"; and {result.Failures.Count - Listed:N0} more" : "";
            Console.Error.WriteLine(
                $"WARNING: {result.Failures.Count:N0} temporary files or folders could not be removed ({shown}{more}). The install is complete; delete '{_paths.StagingRoot}' by hand to free the space.");
            InstallProgress.SetDetail(phase, $"{result.Failures.Count:N0} could not be removed");
        }

        private void RunGameDataStep(int phase, string detail, Action step)
        {
            _ct.ThrowIfCancellationRequested();
            InstallProgress.SetDetail(phase, detail);
            step();
            _ct.ThrowIfCancellationRequested();
            InstallProgress.Tick(phase);
        }

        private void RunFinalizeStep(int phase, string detail, Action step)
        {
            _ct.ThrowIfCancellationRequested();
            InstallProgress.SetDetail(phase, detail);
            step();
            InstallProgress.Tick(phase);
        }

        private void CopyEsm()
        {
            var source = new FileInfo(_paths.GameEsm);
            var local = new FileInfo(_paths.StagedEsm);
            if (local.Exists && local.Length == source.Length && local.LastWriteTimeUtc == source.LastWriteTimeUtc) return;
            Console.WriteLine("Copying Fallout4.esm to staging...");
            File.Copy(_paths.GameEsm, _paths.StagedEsm, overwrite: true);
            File.SetLastWriteTimeUtc(_paths.StagedEsm, source.LastWriteTimeUtc);
        }
    }
}
