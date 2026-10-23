using System.Diagnostics;
using System.Globalization;
using System.IO.Compression;
using AssetConverter;
using AssetConverter.Audio;
using AssetConverter.InstallerUi;
using AssetConverter.Logging;
using AssetConverter.PapyrusLua;
using AssetConverter.Pipeline;
using AssetConverter.Progress;
using AssetConverter.ShaderTranslation.Installation;
using AssetConverter.UiLayout;
using AssetConverter.Video;
using Microsoft.Data.Sqlite;

ParentConsole.Attach();

// Single entrypoint. Running this leaves the system in a state where the user
// can just launch the engine. Idempotent: skips BA2
// extractions whose output directories are already populated.

// Source: Fallout 4 install. OPENFO4_FO4_DIR overrides (benchmarks / users
// with the game elsewhere). The headless install and the dev subcommands use these
// two locations as they are; the installer window asks for both (it starts with
// OPENFO4_FO4_DIR as the game folder when that is set).
const string DevGameFolder = "Z:\\Games\\Fallout 4";
string? fo4Override = Environment.GetEnvironmentVariable("OPENFO4_FO4_DIR");
string fo4Location = fo4Override ?? DevGameFolder;

// Install root; the layout below it is docs/install-contract.md's. A packaged installer has no dev install folder:
// it installs where the window or --install-dir says, and the dev subcommands below are not available in it.
var locations = new InstallLocations(fo4Location, InstallerPaths.DevInstallFolder ?? "");
string stagingRoot = locations.StagingRoot;
string assetsRoot = locations.AssetsRoot;
string meshesOut = locations.MeshesOut;
string texturesOut = locations.TexturesOut;
string dataOut = locations.DataOut;
string fontsOut = locations.FontsOut;
string pipboyArtOut = locations.PipboyArtOut;
string uiLayoutOut = locations.UiLayoutOut;
string settingsDirectory = Environment.GetEnvironmentVariable(LaunchArguments.SettingsDirectoryVariable) ?? GameIniFiles.DocumentsDirectory();

// The dev subcommands share one budget, announced the first time a command that reserves memory asks for it.
var devMemory = new Lazy<MemoryBudget>(() =>
{
    var budget = MemoryBudget.FromSystemLimit();
    Console.WriteLine(budget.Describe());
    return budget;
});

// Dev subcommands for the Papyrus transpiler. Not part of the main pipeline.
// Usage:
//   installer pex-dump <path-to.pex>            -> JSON dump of parsed model
//   installer pex-parse-all <scripts-dir>       -> parse every .pex, write summary log
//   installer pex-emit <path-to.pex>            -> emit skeletal Lua to stdout
//   installer pex-emit-all <scripts-dir> <out>  -> transpile every .pex to <out>/
if (args.Length > 0 && InstallerPaths.IsPackaged && !args[0].StartsWith("--", StringComparison.Ordinal))
{
    Console.Error.WriteLine($"ERROR: '{args[0]}' is a development command; the packaged installer takes only options such as {LaunchArguments.HeadlessFlag}.");
    Environment.ExitCode = InstallSession.ExitFailed;
    return;
}
if (args.Length > 0)
{
    switch (args[0])
    {
        case "pex-dump":
            if (args.Length < 2) { Console.Error.WriteLine("pex-dump requires a path"); return; }
            PapyrusLuaTranspiler.DumpPexAsJson(args[1]);
            return;
        case "pex-parse-all":
            if (args.Length < 2) { Console.Error.WriteLine("pex-parse-all requires a scripts directory"); return; }
            PapyrusLuaTranspiler.ParseAllPex(args[1], Path.Combine(Environment.CurrentDirectory, "papyrus_reader.log"));
            return;
        case "pex-emit":
            if (args.Length < 2) { Console.Error.WriteLine("pex-emit requires a path"); return; }
            PapyrusLuaTranspiler.EmitLua(args[1]);
            return;
        case "pex-emit-all":
            if (args.Length < 3) { Console.Error.WriteLine("pex-emit-all requires <scripts-dir> <out-dir>"); return; }
            PapyrusLuaTranspiler.ConvertToLuaBatched(args[1], args[2], Path.Combine(Environment.CurrentDirectory, "papyrus_lua.log"));
            return;
        case "nif-convert":
            // Dev-only single-file conversion (no BGSM cache / texture index).
            if (args.Length < 3) { Console.Error.WriteLine("nif-convert requires <src.nif> <dest.glb> [materials-root]"); return; }
            if (args.Length > 3) MaterialCensus.Build(args[3]);
            InstallOperations.UseEffectShaderCatalog(fo4Location);
            NifConverter.ConvertToGltf(args[1], args[2]);
            return;
        case "bake-palette":
            // Dev-only re-run of the greyscale-to-palette bake (step 2b)
            // against an existing deployment, without reconverting meshes.
            // New-layout installs have no staging\textures — sources live in
            // assets\textures (the extracted originals double as sources).
            if (!Directory.Exists(Path.Combine(stagingRoot, "textures")))
                PaletteBaker.SourceTexturesOverride = texturesOut;
            PaletteBaker.BakeAll(stagingRoot, texturesOut);
            return;
        case "meshes-only":
            {
                // Dev-only re-run of NIF -> glTF conversion (steps 2 + 2c) against
                // an existing extraction, without touching textures or the DB.
                // Collision sidecars survive the GLB purge and skip-existing makes
                // 2c a near no-op; LOD backfill stays valid because the same
                // _LOD.glb set is regenerated.
                var tMeshOnly = Stopwatch.StartNew();
                PreloadTextureIndexFromDeployIfNeeded(stagingRoot, texturesOut);
                InstallOperations.PurgeByExtension(meshesOut, "*.glb", "GLB meshes");
                InstallOperations.UseEffectShaderCatalog(fo4Location);
                NifConverter.ConvertToGltfBatched(Path.Combine(stagingRoot, "Meshes"), meshesOut, bgsmScanRoot: stagingRoot, memory: devMemory.Value);
                ExtractCollisionFromStaging(stagingRoot, meshesOut, devMemory.Value);
                HavokAnimationStage.ExtractActorHkx(locations, memory: devMemory.Value);
                HavokAnimationStage.Convert(stagingRoot, meshesOut, memory: devMemory.Value);
                Console.WriteLine($"Meshes reconverted in {ProgressFormatter.Invariant.Duration(tMeshOnly.Elapsed)}");
                return;
            }
        case "animations-only":
            {
                // Dev-only re-run of the Havok animation stage: extracts the actor
                // .hkx files from Animations.ba2 (skipped when already staged) and
                // rewrites the skeleton / clip-pack GLBs under assets\meshes\Actors.
                var tAnimOnly = Stopwatch.StartNew();
                Directory.CreateDirectory(stagingRoot);
                HavokAnimationStage.ExtractActorHkx(locations, memory: devMemory.Value);
                HavokAnimationStage.Convert(stagingRoot, meshesOut, memory: devMemory.Value);
                Console.WriteLine($"Animations converted in {ProgressFormatter.Invariant.Duration(tAnimOnly.Elapsed)}");
                return;
            }
        case "collision-only":
            {
                // Dev-only re-run of collision sidecar extraction (step 2c) against
                // an existing GLB deployment, without reconverting meshes. The staged
                // meshes are made ready first (extracted again when the staging folder
                // is gone); only then is the existing .col.json set purged, because
                // ExtractFo4Collision always passes --skip-existing and a warm sidecar
                // would never pick up a change of the extractor.
                var tColOnly = Stopwatch.StartNew();
                new PipelineStages(locations, devMemory.Value, CancellationToken.None).EnsureStagedMeshArchives();
                CollisionSidecars.Purge(meshesOut);
                ExtractCollisionFromStaging(stagingRoot, meshesOut, devMemory.Value);
                Console.WriteLine($"Collision sidecars regenerated in {ProgressFormatter.Invariant.Duration(tColOnly.Elapsed)}");
                return;
            }
        case "addn-only":
            {
                // Dev-only refill of addon_nodes (ADDN records) in the existing
                // database; same as `esm-only --tables addon_nodes`.
                ReloadEsmTables("addon_nodes", Path.Combine(dataOut, "data.db"), ResolveFallout4Esm(stagingRoot, fo4Location),
                    fo4Location, settingsDirectory, stagingRoot, texturesOut, meshesOut, devMemory.Value);
                return;
            }
        case "esm-only":
            {
                // Dev-only DB rebuild (steps 3 + 3a) against an existing
                // deployment: re-runs ESM -> SQLite + MODC swaps + LOD backfill in minutes
                // without purging/reconverting GLBs or textures.
                // Usage: installer esm-only [--tables <table>[,<table>...]]
                // With --tables only those tables are cleared and re-filled in the
                // existing database (see ESMLoader.ReloadTablesAsync).
                string dbOnlyPath = Path.Combine(dataOut, "data.db");
                string esmOnlySrc = ResolveFallout4Esm(stagingRoot, fo4Location);

                if (!EsmOnlyArguments.TryParse(args, out string? tablesCsv))
                {
                    Console.Error.WriteLine("Usage: esm-only [--tables <table>[,<table>...]]");
                    Environment.ExitCode = 1;
                    return;
                }
                if (tablesCsv != null)
                {
                    ReloadEsmTables(tablesCsv, dbOnlyPath, esmOnlySrc, fo4Location, settingsDirectory, stagingRoot, texturesOut, meshesOut, devMemory.Value);
                    return;
                }

                if (File.Exists(dbOnlyPath))
                {
                    Console.WriteLine("Deleting existing database for fresh rebuild...");
                    File.Delete(dbOnlyPath);
                    foreach (var aux in new[] { dbOnlyPath + "-shm", dbOnlyPath + "-wal" })
                        if (File.Exists(aux)) File.Delete(aux);
                }
                if (!Directory.Exists(Path.Combine(stagingRoot, "textures")))
                    PaletteBaker.SourceTexturesOverride = texturesOut;
                PreloadTextureIndexFromDeployIfNeeded(stagingRoot, texturesOut);
                ESMLoader esmOnly = new(dbOnlyPath, materialsRoot: stagingRoot, texturesOut: texturesOut)
                {
                    Strings = StringTables.Extract(fo4Location, stagingRoot, GameLanguage.Resolve(fo4Location, settingsDirectory)),
                    Memory = devMemory.Value,
                };
                esmOnly.LoadESM(esmOnlySrc);
                ModcSwapSynthesizer.Run(dbOnlyPath, meshesOut, stagingRoot, texturesOut);
                LodWaterStage.Run(Path.Combine(fo4Location, "Data"), dbOnlyPath);
                LodBackfill.Run(dbOnlyPath, meshesOut);
                ModelPathNormalizer.Run(dbOnlyPath);
                Console.WriteLine($"Database rebuilt at {dbOnlyPath}");
                return;
            }
        case "lod-water-only":
            {
                // Dev-only refill of lod_water_quads from the game's terrain LOD (BTR) files.
                // Needs an existing database whose worldspaces table is filled.
                LodWaterStage.Run(Path.Combine(fo4Location, "Data"), Path.Combine(dataOut, "data.db"));
                return;
            }
        case "normalize-paths":
            // Dev-only re-run of step 3b against an existing deployment's DB,
            // without rebuilding anything else.
            ModelPathNormalizer.Run(Path.Combine(dataOut, "data.db"));
            return;
        case "inis-only":
            // Dev-only re-run of step 2d against an existing deployment.
            InstallOperations.CopyGameInis(fo4Location, dataOut, settingsDirectory);
            return;
        case "fonts-only":
            // Dev-only re-run of the UI-font extraction (Interface.ba2 ->
            // fonts_en.swf -> ffdec -> assets\interface\fonts).
            RunConversionCommand(() => FontExtractor.Extract(fo4Location, settingsDirectory, stagingRoot, fontsOut, devMemory.Value));
            return;
        case "ui-layout-only":
            // Dev-only re-run of the UI layout extraction (Interface.ba2 -> every interface movie, translate table and font
            // configuration of the installed language -> assets\interface\layout\*.json); current documents are kept.
            RunConversionCommand(() => UiLayoutExtractor.Extract(fo4Location, settingsDirectory, uiLayoutOut));
            return;
        case "pipboy-art-only":
            // Dev-only re-run of the Pip-Boy art export (Interface.ba2 -> movies -> ffdec -> SVG frames + art_manifest.json).
            RunConversionCommand(() => PipboyArtExtractor.Extract(fo4Location, stagingRoot, pipboyArtOut, devMemory.Value));
            return;
        case "audio-only":
            // Dev-only re-run of the audio conversion (Sounds.ba2 + Startup.ba2 -> ffmpeg -> assets\audio\**.flac). Needs no staging.
            // Everything is converted again; the plan (tool, archives, entries) is made before any existing output is touched, so a failing plan costs nothing.
            RunConversionCommand(() =>
            {
                new PipelineStages(locations, devMemory.Value, CancellationToken.None).ConvertAudio(force: true);
                return true;
            });
            return;
        case "video-only":
            // Dev-only re-run of the video conversion (Data\Video\*.bk2 -> Bink2 decoder -> ffmpeg -> assets\video\*.mp4). Needs no staging.
            // Everything is converted again; the plan (tools, folder, videos) is made before any existing output is touched.
            RunConversionCommand(() =>
            {
                new PipelineStages(locations, devMemory.Value, CancellationToken.None).ConvertVideo(force: true);
                return true;
            });
            return;
        case "shader-translate":
            // Dev-only re-run of the shader translation step: Shaders.ba2 -> assets\shaders (WGSL + manifest.json).
            ShaderTranslateStep.Run(fo4Location, assetsRoot, InstallerPaths.Require(ShaderManifestReader.RepoRelativePath));
            return;
        case "bench-ba2":
            {
                // Dev/bench-only: extract one BA2 with either the legacy serial
                // unpacker or the pipelined parallel one, into a throwaway dir.
                // Usage: installer bench-ba2 <archive.ba2> <outDir> <serial|parallel> [dop]
                if (args.Length < 4) { Console.Error.WriteLine("bench-ba2 requires <archive> <outDir> <serial|parallel> [dop]"); return; }
                var benchSw = Stopwatch.StartNew();
                if (args[2] == "-") { Console.Error.WriteLine("outDir required"); return; }
                Directory.CreateDirectory(args[2]);
                if (args[3] == "serial")
                {
                    BA2Unpacker.Unpack(args[1], args[2]);
                }
                else
                {
                    int? dop = args.Length > 4 ? int.Parse(args[4], CultureInfo.InvariantCulture) : null;
                    BA2ParallelUnpacker.Unpack(args[1], args[2], devMemory.Value, dop);
                }
                benchSw.Stop();
                Console.WriteLine($"BENCH bench-ba2 mode={args[3]} archive={Path.GetFileName(args[1])} elapsed={benchSw.Elapsed.TotalSeconds:F1}s");
                return;
            }
        case "bench-nif":
            {
                // Dev/bench-only: NIF -> GLB conversion against the existing
                // staging extraction, into a throwaway output dir. Worker count /
                // file cap / GLB validation come from env vars (see nif_converter).
                // Usage: installer bench-nif <outDir>
                if (args.Length < 2) { Console.Error.WriteLine("bench-nif requires <outDir>"); return; }
                var benchSw = Stopwatch.StartNew();
                PreloadTextureIndexFromDeployIfNeeded(stagingRoot, texturesOut);
                InstallOperations.UseEffectShaderCatalog(fo4Location);
                NifConverter.ConvertToGltfBatched(Path.Combine(stagingRoot, "Meshes"), args[1], bgsmScanRoot: stagingRoot, memory: devMemory.Value);
                benchSw.Stop();
                Console.WriteLine($"BENCH bench-nif elapsed={benchSw.Elapsed.TotalSeconds:F1}s workers={Environment.GetEnvironmentVariable("OPENFO4_NIF_WORKERS") ?? "default"} maxfiles={Environment.GetEnvironmentVariable("OPENFO4_NIF_MAXFILES") ?? "all"} validate={Environment.GetEnvironmentVariable("OPENFO4_GLB_VALIDATE") ?? "default"} gc={(System.Runtime.GCSettings.IsServerGC ? "server" : "workstation")}");
                return;
            }
        case "bench-esm":
            {
                // Dev/bench-only: ESM -> SQLite into a throwaway DB path.
                // Usage: installer bench-esm <dbPath> [esmPath]
                if (args.Length < 2) { Console.Error.WriteLine("bench-esm requires <dbPath> [esmPath]"); return; }
                string benchDb = args[1];
                string benchEsm = args.Length > 2 ? args[2] : Path.Combine(fo4Location, "Data", "Fallout4.esm");
                foreach (var f in new[] { benchDb, benchDb + "-shm", benchDb + "-wal" })
                    if (File.Exists(f)) File.Delete(f);
                var benchSw = Stopwatch.StartNew();
                ESMLoader benchLoader = new(benchDb, materialsRoot: stagingRoot, texturesOut: texturesOut)
                {
                    Strings = StringTables.Extract(fo4Location, stagingRoot, GameLanguage.Resolve(fo4Location, settingsDirectory)),
                    Memory = devMemory.Value,
                };
                benchLoader.LoadESM(benchEsm);
                benchSw.Stop();
                Console.WriteLine($"BENCH bench-esm elapsed={benchSw.Elapsed.TotalSeconds:F1}s esm={benchEsm} gc={(System.Runtime.GCSettings.IsServerGC ? "server" : "workstation")}");
                return;
            }
        case "movt-only":
            {
                // Dev-only re-extract of MOVT records (90 rows) into an existing
                // deployment's DB, without rebuilding anything else.
                string movtDb = Path.Combine(dataOut, "data.db");
                if (!File.Exists(movtDb)) { Console.Error.WriteLine($"No database at {movtDb}; run esm-only first."); return; }
                using var movtMod = Mutagen.Bethesda.Fallout4.Fallout4Mod.CreateFromBinaryOverlay(
                    Path.Combine(fo4Location, "Data", "Fallout4.esm"), Mutagen.Bethesda.Fallout4.Fallout4Release.Fallout4);
                using var movtConn = DatabaseManager.OpenWorkerConnection(movtDb);
                new MovementTypeProcessor().ProcessRecordsAsync(movtMod.MovementTypes, "MOVT", movtConn).GetAwaiter().GetResult();
                return;
            }
    }
}

LaunchArguments launch;
try
{
    launch = LaunchArguments.Parse(args, Environment.GetEnvironmentVariable);
}
catch (LaunchArgumentException ex)
{
    Console.Error.WriteLine($"ERROR: {ex.Message}");
    Environment.ExitCode = InstallSession.ExitFailed;
    return;
}
InstallSession session;
InstallOptions options;
if (launch.Headless)
{
    try
    {
        locations = HeadlessLocations.Resolve(launch, fo4Override, InstallerPaths.IsPackaged ? null : DevGameFolder, InstallerPaths.DevInstallFolder);
    }
    catch (LaunchArgumentException ex)
    {
        Console.Error.WriteLine($"ERROR: {ex.Message}");
        Environment.ExitCode = InstallSession.ExitFailed;
        return;
    }
    options = new InstallOptions(launch.KeepsStagingWhenHeadless, UnpacksPayload: InstallerPaths.IsPackaged);
    if (GameIniFiles.MissingError(locations.Fo4Location, locations.SettingsDirectory) is { } missingIni)
    {
        Console.Error.WriteLine($"ERROR: {missingIni}");
        Environment.ExitCode = InstallSession.ExitFailed;
        return;
    }
    PrepareInstallFolderOrExit(locations);
    session = InstallSession.StartOrExit(headless: true, locations.LogFile, options);
}
else
{
    var setup = new SetupViewModel(
        launch.GameDirectory ?? fo4Override ?? InstallFolderRules.DefaultGameFolder(SteamGameFolderLocator.FromRegistry(), note => Console.WriteLine($"Game folder detection: {note}")) ?? "", launch.KeepStaging,
        launch.SettingsDirectory ?? GameIniFiles.DocumentsDirectory());
    var viewModel = new InstallerViewModel(setup);
    var window = InstallerWindowHost.StartOrExit(viewModel);
    SetupChoice? choice = await setup.Completed;
    if (choice == null)
    {
        window.Shutdown();
        await window.Closed;
        Environment.ExitCode = InstallSession.ExitCancelled;
        return;
    }
    locations = new InstallLocations(choice.GameFolder, choice.InstallFolder, choice.SettingsFolder);
    options = new InstallOptions(choice.KeepStaging, UnpacksPayload: InstallerPaths.IsPackaged);
    if (!InstallSession.TryStartInOpenWindow(viewModel, window, locations.LogFile, options, out var startedSession))
    {
        await window.Closed;
        Environment.ExitCode = InstallSession.ExitFailed;
        return;
    }
    session = startedSession;
}
using var activeSession = session;

Exception? ended = null;
try
{
    var resources = await PayloadUnpackStep.RunAsync(InstallerPaths.IsPackaged, InstallerPaths.Unpack);
    options = options with { Game = resources.Game };
    await new InstallPipeline(locations, options, MemoryBudget.FromSystemLimit(), InstallProgress.CancelToken).RunAsync();
}
catch (Exception ex)
{
    ended = ex;
}
Environment.ExitCode = session.Finish(ended);

// A dev conversion command that meets malformed game data reports it like a failed install step: the message and a failure exit code.
static void RunConversionCommand(Func<bool> convert)
{
    try
    {
        convert();
    }
    catch (ConversionStepException ex)
    {
        Console.Error.WriteLine($"ERROR: {ex.Message}");
        Environment.ExitCode = InstallSession.ExitFailed;
    }
}

static void PrepareInstallFolderOrExit(InstallLocations locations)
{
    string? error = InstallFolderPreparation.Prepare(locations);
    if (error != null) Environment.Exit(InstallSession.ReportStartupFailure(new InstallStartupException(error), locations.HoldsInstall() ? locations.LogFile : null));
}

// Dev subcommands run against installs whose textures were extracted straight
// into assets\textures (no staging\textures tree): seed the converter's
// texture-existence index from the deployed files instead.
static void PreloadTextureIndexFromDeployIfNeeded(string stagingRoot, string texturesOut)
{
    if (Directory.Exists(Path.Combine(stagingRoot, "textures"))) return;
    if (!Directory.Exists(texturesOut)) return;
    NifConverter.PreloadTextureIndex(
        Directory.EnumerateFiles(texturesOut, "*.dds", SearchOption.AllDirectories)
            .Select(f => Path.GetRelativePath(texturesOut, f)));
    AssetConverter.MaterialCensus.TextureExistsOverride = NifConverter.TextureIndexContains;
}

// Dev subcommands know nothing about which NIFs carry collision: every staged NIF goes to the worker, whose own prefilter decides.
static void ExtractCollisionFromStaging(string stagingRoot, string meshesOut, MemoryBudget memory)
{
    string stagedMeshes = Path.Combine(stagingRoot, "Meshes");
    var stagedNifs = NifConverter.EnumerateNifs(stagedMeshes).Order(StringComparer.OrdinalIgnoreCase).ToArray();
    CollisionSidecars.PurgeIfStale(meshesOut);
    NifConverter.ExtractFo4Collision(stagedMeshes, meshesOut, stagedNifs, memory);
    CollisionSidecars.RecordCurrent(meshesOut);
}

// The staged copy of Fallout4.esm when present, else the game's own copy.
static string ResolveFallout4Esm(string stagingRoot, string fo4Location)
{
    string staged = Path.Combine(stagingRoot, "Fallout4.esm");
    return File.Exists(staged) ? staged : Path.Combine(fo4Location, "Data", "Fallout4.esm");
}

// Partial counterpart of esm-only: re-fills only the given tables of the
// existing database, then runs the post-passes that touch those tables.
static void ReloadEsmTables(string tablesCsv, string dbPath, string esmSource, string fo4Location, string settingsDirectory,
    string stagingRoot, string texturesOut, string meshesOut, MemoryBudget memory)
{
    var requested = EsmOnlyArguments.SplitTables(tablesCsv);
    TableReloadPlan plan;
    try { plan = TableReloadPlan.Create(requested); }
    catch (TableSelectionException e) { FailReload(e.Message); return; }

    if (!File.Exists(esmSource)) { FailReload($"Fallout4.esm not found at {esmSource}."); return; }

    if (plan.ReloadsMaterialSwaps)
    {
        if (!Directory.Exists(Path.Combine(stagingRoot, "textures")))
            PaletteBaker.SourceTexturesOverride = texturesOut;
        PreloadTextureIndexFromDeployIfNeeded(stagingRoot, texturesOut);
    }

    ESMLoader loader = new(dbPath, materialsRoot: stagingRoot, texturesOut: texturesOut)
    {
        Strings = StringTables.Extract(fo4Location, stagingRoot, GameLanguage.Resolve(fo4Location, settingsDirectory)),
        Memory = memory,
    };
    try { loader.ReloadTablesAsync(esmSource, plan).GetAwaiter().GetResult(); }
    catch (Exception e) when (e is MissingDatabaseException or MissingTableException) { FailReload(e.Message); return; }
    catch (SqliteException e)
    {
        FailReload($"Database error, the transaction was rolled back and the previous data is intact: {e.Message}");
        return;
    }

    // These run on their own connections after the commit, so a failure leaves
    // the reloaded tables in place but the post-step undone.
    if (plan.ReloadsMaterialSwaps &&
        !RunReloadPostStep("MODC synthesis", "rerun `esm-only --tables material_swaps` to retry",
            () => ModcSwapSynthesizer.Run(dbPath, meshesOut, stagingRoot, texturesOut))) return;
    if (plan.Tables.Contains("statics") &&
        !RunReloadPostStep("LOD backfill", "rerun `esm-only --tables statics` to retry",
            () => LodBackfill.Run(dbPath, meshesOut))) return;
    if (!RunReloadPostStep("path normalization", "rerun `normalize-paths` to retry",
            () => ModelPathNormalizer.Run(dbPath, plan.Tables))) return;
    Console.WriteLine($"Tables reloaded in {dbPath}: {string.Join(", ", plan.Tables)}");
}

static bool RunReloadPostStep(string name, string retryHint, Action step)
{
    try { step(); return true; }
    catch (Exception e) when (e is SqliteException or IOException)
    {
        FailReload($"Tables were reloaded, but post-step '{name}' failed: {e.Message}. To finish, {retryHint}.");
        return false;
    }
}

static void FailReload(string message)
{
    Console.Error.WriteLine(message);
    Environment.ExitCode = 1;
}
