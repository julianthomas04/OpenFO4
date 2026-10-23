using System.Collections.Concurrent;
using System.ComponentModel;
using System.Diagnostics;
using System.Globalization;
using System.Numerics;
using System.Text.Json;
using System.Text.Json.Nodes;
using System.Text.Json.Serialization;
using AssetConverter.Pipeline;
using AssetConverter.Progress;
using NiflySharp;
using NiflySharp.Blocks;
using NiflySharp.Structs;
using SharpGLTF.Geometry;
using SharpGLTF.Geometry.VertexTypes;
using SharpGLTF.Materials;
using SharpGLTF.Scenes;
using SharpGLTF.Schema2;

namespace AssetConverter
{
    public class NifConverter
    {

        private static ConcurrentDictionary<string, byte> _missingTextures = new ConcurrentDictionary<string, byte>();
        private static ConcurrentDictionary<string, byte> _unsupportedShaders = new ConcurrentDictionary<string, byte>();
        // File-level failures: one entry per NIF that produced no output.
        private static ConcurrentBag<string> _errors = new ConcurrentBag<string>();
        // Per-shape / per-clip problems inside a NIF that still converted.
        private static ConcurrentBag<string> _warnings = new ConcurrentBag<string>();

        // Shape-level categorization (see Phase A of texture-plan). Every shape the converter
        // touches increments exactly one of these so we can tell where texture data is lost.
        private static int _shapesTotal;
        private static int _shapesNoShaderRef;
        private static int _shapesLightingInlineTextures;
        private static int _shapesLightingBgsmResolved;
        private static int _shapesLightingBgsmReference;
        private static int _shapesLightingEmpty;
        private static int _shapesEffectShader;
        private static int _shapesEffectBgemResolved;
        private static int _shapesEffectUnresolved;
        private static int _shapesWaterShader;
        private static int _shapesOtherShader;
        private static int _shapesInlineEnvCubeMissing;
        private static int _tangentVertices;
        private static int _tangentUvGradientVertices;
        private static int _tangentDegenerateVertices;

        // BGSM/BGEM lookups live in MaterialCensus (shared with PaletteBaker and
        // the ESM loader's material-swap resolution); built before conversion starts.

        // Existence index of extracted texture files, keyed by path relative to the
        // Textures root (lowercase, forward-slash). _texturesByName maps bare filename
        // -> full relative paths, for NIFs that reference a texture without its
        // directory. Built by BuildTextureIndex before conversion; read-only after.
        private static HashSet<string> _textureIndex = new();
        private static Dictionary<string, List<string>> _texturesByName = new();
        private static int _texPathsRepaired;

        // CK placeholder textures referenced by some vanilla NIFs (inline TextureSets
        // with no BGSM) that never shipped in any BA2. Substitute the closest shipped
        // flat texture rather than letting the engine fall back to magenta.
        private static readonly Dictionary<string, string> _placeholderTextures = new()
        {
            ["gray.dds"] = "shared/flatgray01_d.dds",
            ["darkgray.dds"] = "shared/black01_d.dds",
            ["default_n.dds"] = "shared/flatflat_n.dds",
        };

        // BSWaterShaderProperty shapes carry no diffuse of their own (vanilla water
        // is procedural). Until the engine grows a water shader, water borrows these
        // shipped FO4 water textures so it renders as hidden FX instead of the magenta
        // missing-material fallback (see the water branch in GetMaterial).
        private const string _waterFxDiffuse = @"Water\DefaultWaterTile.dds";
        private const string _waterFxNormal = @"Water\WaterTileHumpy_n.dds";

        // Debug samples: first few distinct shader.Name values seen for each outcome,
        // so we can diagnose lookup misses without dumping 27k strings.
        private static ConcurrentDictionary<string, byte> _bgsmRefSamplesResolved = new();
        private static ConcurrentDictionary<string, byte> _bgsmRefSamplesUnresolved = new();
        private static ConcurrentDictionary<string, byte> _bgemRefSamplesUnresolved = new();
        private const int _bgsmSampleCap = 20;

        // GLB write settings. OPENFO4_GLB_VALIDATE=skip drops SharpGLTF's
        // per-file strict validation DOM walk (bench hook; output bytes are
        // identical for models that would have validated).
        private static readonly SharpGLTF.Schema2.WriteSettings _writeSettings = new()
        {
            Validation = Environment.GetEnvironmentVariable("OPENFO4_GLB_VALIDATE") == "skip"
                ? SharpGLTF.Validation.ValidationMode.Skip
                : SharpGLTF.Validation.ValidationMode.Strict,
        };

        /// <summary>Every .nif under <paramref name="source"/>; the enumeration of a full game install is long enough to be shared between callers.</summary>
        public static string[] EnumerateNifs(string source) => Directory.GetFiles(source, "*.nif", SearchOption.AllDirectories);

        /// <summary>Converts the given files, or every .nif under <paramref name="source"/> when the caller did not enumerate it already (see <see cref="EnumerateNifs"/>).</summary>
        internal static int ConvertToGltfBatched(string source, string destination, MemoryBudget memory, string? bgsmScanRoot = null,
            int progressPhase = -1, IReadOnlyList<string>? nifFiles = null, CancellationToken ct = default)
        {
            var work = new NifWorkQueue();
            work.AddExisting(nifFiles ?? EnumerateNifs(source));
            work.Complete();
            return ConvertToGltfBatched(source, destination, work, memory, bgsmScanRoot, progressPhase, priorityFiles: null, ct);
        }

        /// <summary>
        /// Converts the files of <paramref name="work"/> as they arrive; the call returns once its producers are done and every file is
        /// converted. Static statistics are reset once, here.
        /// </summary>
        /// <param name="work">The files to convert; <see cref="NifWorkQueue.ExpectedCount"/> is the total the progress is reported against.</param>
        /// <param name="priorityFiles">Counts the priority files of <paramref name="work"/> down as they are converted; its signal is released when the queue is drained.</param>
        /// <param name="memory">Admits each file's conversion against the install's memory.</param>
        /// <returns>How many files were converted or failed.</returns>
        internal static int ConvertToGltfBatched(string source, string destination, NifWorkQueue work, MemoryBudget memory, string? bgsmScanRoot = null,
            int progressPhase = -1, PriorityFiles? priorityFiles = null, CancellationToken ct = default)
        {
            ResetStatistics();

            MaterialCensus.Build(bgsmScanRoot ?? source);
            BuildTextureIndex(Path.Combine(bgsmScanRoot ?? source, "textures"));

            var (maxFiles, workerCount) = ReadBenchOverrides();
            int overlapWorkers = NifConversionWorkers.OverlapWorkerCount(Environment.ProcessorCount, workerCount);

            int expected = Math.Min(work.ExpectedCount, maxFiles);
            Console.WriteLine($"Converting {expected} nif files to glTF ({workerCount} workers" +
                (work.IsComplete ? ")." : $", {overlapWorkers} of them while the archives are still extracting)."));

            Stopwatch sw = new Stopwatch();
            sw.Start();

            int totalProcessed = 0;
            object consoleLock = new object();

            // Created-directory cache: the corpus shares a few thousand
            // directories across 160k files; skip the per-file syscall.
            var createdDirs = new ConcurrentDictionary<string, byte>(StringComparer.OrdinalIgnoreCase);

            // Convert and write each file immediately: keeps models
            // gen0/gen1 instead of buffering hundreds into gen2, and
            // overlaps CPU with write I/O.
            void ConvertOne(string file)
            {
                InstallProgress.SetDetail(progressPhase, Path.GetFileName(file));
                string? dest = null;
                try
                {
                    using var reservation = memory.Reserve(MemoryEstimates.NifConversionBytes(InputLength(file)), ct);
                    try
                    {
                        var model = ConvertNifToGltf(file);
                        if (model != null)
                        {
                            string relative = Path.GetRelativePath(source, file);
                            dest = Path.Combine(destination, Path.ChangeExtension(relative, ".glb"));
                            // CreateDirectory before recording: TryAdd-then-create
                            // would let another worker see the cache hit while the
                            // directory is still being created and fail its save.
                            string dir = Path.GetDirectoryName(dest)!;
                            if (!createdDirs.ContainsKey(dir))
                            {
                                Directory.CreateDirectory(dir);
                                createdDirs.TryAdd(dir, 0);
                            }
                            model.SaveGLB(dest, _writeSettings);
                        }
                    }
                    catch (Exception ex)
                    {
                        _errors.Add($"Error converting {file} -> {dest}: {ex.Message}");
                    }
                }
                finally
                {
                    priorityFiles?.Converted(file);
                }

                int current = Interlocked.Increment(ref totalProcessed);
                InstallProgress.Tick(progressPhase);
                if (current % 4096 == 0)
                {
                    lock (consoleLock)
                    {
                        Console.WriteLine($"  {current}/{expected} ({(100.0 * current / expected):F1}%) — {sw.Elapsed:mm\\:ss} elapsed");
                    }
                }
            }

            NifConversionWorkers.Run(work, overlapWorkers, workerCount, ConvertOne, maxFiles, ct);
            priorityFiles?.Release();

            sw.Stop();
            Console.WriteLine($"Done. {totalProcessed - _errors.Count} succeeded, {_errors.Count} failed, {_warnings.Count} warnings in {sw.Elapsed:mm\\:ss}.");

            WriteSummaryLog(totalProcessed);
            ReadFailures.WriteSummaryAndReset(Console.Out);
            return totalProcessed;
        }

        /// <summary>The size of the file, or zero when it is missing, which the conversion then reports as that file's own failure.</summary>
        private static long InputLength(string file) => new FileInfo(file) is { Exists: true } info ? info.Length : 0;

        /// <summary>
        /// Bench hooks (default behavior unchanged when unset): OPENFO4_NIF_MAXFILES converts only the first N files taken from the
        /// queue, OPENFO4_NIF_WORKERS overrides the worker count.
        /// </summary>
        private static (int MaxFiles, int Workers) ReadBenchOverrides()
        {
            int maxFiles = int.TryParse(Environment.GetEnvironmentVariable("OPENFO4_NIF_MAXFILES"), out int mf) && mf > 0 ? mf : int.MaxValue;
            int workers = int.TryParse(Environment.GetEnvironmentVariable("OPENFO4_NIF_WORKERS"), out int wc) && wc > 0 ? wc : Environment.ProcessorCount;
            return (maxFiles, workers);
        }

        private static void ResetStatistics()
        {
            _missingTextures.Clear();
            _unsupportedShaders.Clear();
            _errors = new ConcurrentBag<string>();
            _warnings = new ConcurrentBag<string>();
            ReadFailures = new ReadFailureTally();
            _shapesTotal = 0;
            _shapesNoShaderRef = 0;
            _shapesLightingInlineTextures = 0;
            _shapesLightingBgsmResolved = 0;
            _shapesLightingBgsmReference = 0;
            _shapesLightingEmpty = 0;
            _shapesEffectShader = 0;
            _shapesEffectBgemResolved = 0;
            _shapesEffectUnresolved = 0;
            _shapesWaterShader = 0;
            _shapesOtherShader = 0;
            _shapesInlineEnvCubeMissing = 0;
            _tangentVertices = 0;
            _tangentUvGradientVertices = 0;
            _tangentDegenerateVertices = 0;
            _bgsmRefSamplesResolved.Clear();
            _bgsmRefSamplesUnresolved.Clear();
            _bgemRefSamplesUnresolved.Clear();
            _texPathsRepaired = 0;
            NifAnimationStats.Reset();
        }

        // FO4 NIFs store collision as a raw Havok 2014.1.0-r1 packfile blob
        // inside bhkPhysicsSystem.BinaryData. NiflySharp exposes it as opaque
        // bytes, so we shell out to PyNifly's bhk_autounpack (in third_party/)
        // via a batch worker (src/installer/python/fo4_collision_worker.py). One process
        // covers all NIFs, so per-file subprocess overhead is ~zero.
        // Kept as a separate entry point so install.cs can opt in explicitly —
        // the sidecars are idempotent and expensive to regenerate on every run.
        /// <param name="nifFiles">The NIFs below <paramref name="sourceDir"/> the worker looks at; ones without collision are rejected by its own prefilter.</param>
        public static void ExtractFo4Collision(string sourceDir, string destDir, IReadOnlyCollection<string> nifFiles, MemoryBudget memory, int progressPhase = -1, CancellationToken ct = default)
        {
            using var list = CollisionNifList.Create(sourceDir, nifFiles);
            CollisionWorkerProcess.RunFromList(sourceDir, destDir, list.FilePath, progressPhase, memory, ct);
        }

        /// <summary>The same extraction for NIFs that are still being queued: the worker starts at once and decodes each as it arrives, until <paramref name="candidates"/> is finished and drained.</summary>
        internal static void ExtractFo4Collision(string sourceDir, string destDir, NifWorkQueue candidates, int progressPhase, MemoryBudget memory, CancellationToken ct) =>
            CollisionWorkerProcess.RunStreaming(sourceDir, destDir, candidates, progressPhase, memory, ct);

        private static void WriteSummaryLog(int fileCount)
        {
            int total = _shapesTotal;
            int lightingInline = _shapesLightingInlineTextures;
            int lightingBgsmResolved = _shapesLightingBgsmResolved;
            int lightingBgsmUnresolved = _shapesLightingBgsmReference;
            int lightingEmpty = _shapesLightingEmpty;
            int effect = _shapesEffectShader;
            int effectBgemResolved = _shapesEffectBgemResolved;
            int effectUnresolved = _shapesEffectUnresolved;
            int water = _shapesWaterShader;
            int other = _shapesOtherShader;
            int noShader = _shapesNoShaderRef;

            Console.WriteLine();
            Console.WriteLine("---- BGSM resolution samples ----");
            Console.WriteLine("Shader Name values that resolved to a cache entry (up to 20):");
            foreach (var s in _bgsmRefSamplesResolved.Keys.OrderBy(k => k))
                Console.WriteLine($"  OK   '{s}'");
            Console.WriteLine("Shader Name values that did NOT match any cache entry (up to 20):");
            foreach (var s in _bgsmRefSamplesUnresolved.Keys.OrderBy(k => k))
                Console.WriteLine($"  MISS '{s}'");
            Console.WriteLine("Effect-shader BGEM references that did NOT match (up to 20):");
            foreach (var s in _bgemRefSamplesUnresolved.Keys.OrderBy(k => k))
                Console.WriteLine($"  MISS '{s}'");
            Console.WriteLine("---------------------------------");
            Console.WriteLine();
            Console.WriteLine("---- NIF conversion summary ----");
            Console.WriteLine($"  Files:                      {fileCount}");
            Console.WriteLine($"  Shapes total:               {total}");
            Console.WriteLine($"  Lighting + inline tex:      {lightingInline}  ({Pct(lightingInline, total)})");
            Console.WriteLine($"  Lighting + BGSM resolved:   {lightingBgsmResolved}  ({Pct(lightingBgsmResolved, total)})");
            Console.WriteLine($"  Lighting + BGSM unresolved: {lightingBgsmUnresolved}  ({Pct(lightingBgsmUnresolved, total)})");
            Console.WriteLine($"  Lighting + empty:           {lightingEmpty}  ({Pct(lightingEmpty, total)})");
            Console.WriteLine($"  Effect shader (total):      {effect}  ({Pct(effect, total)})");
            Console.WriteLine($"    -> BGEM resolved:         {effectBgemResolved}  ({Pct(effectBgemResolved, total)})");
            Console.WriteLine($"    -> unresolved:            {effectUnresolved}  ({Pct(effectUnresolved, total)})");
            Console.WriteLine($"  Water shader (-> FX):       {water}  ({Pct(water, total)})");
            Console.WriteLine($"  Other shader:               {other}  ({Pct(other, total)})");
            Console.WriteLine($"  No shader ref:              {noShader}  ({Pct(noShader, total)})");
            Console.WriteLine($"  Material census entries:    {MaterialCensus.Count}");
            Console.WriteLine($"  Texture paths repaired:     {_texPathsRepaired}");
            Console.WriteLine($"  Env cubes missing (materials / inline shapes): {MaterialCensus.MissingEnvCubeCount} / {_shapesInlineEnvCubeMissing}");
            Console.WriteLine($"  Tangent vertices:           {_tangentVertices}");
            Console.WriteLine($"  WARNING UV-gradient tangents (no/invalid NIF tangents): {_tangentUvGradientVertices}");
            Console.WriteLine($"  WARNING degenerate tangents (arbitrary frame):          {_tangentDegenerateVertices}");
            Console.WriteLine($"  Errors (file-level):        {_errors.Count}");
            Console.WriteLine($"  Warnings (shape/clip-level): {_warnings.Count}");
            foreach (var line in NifAnimationStats.AnimationSummaryLines())
                Console.WriteLine($"  Animation/skin - {line}");
            foreach (var line in NifAnimationStats.EffectSummaryLines())
                Console.WriteLine($"  Effects/particles - {line}");
            Console.WriteLine("--------------------------------");

            try
            {
                string logPath = "nif_converter.log";
                using var writer = new StreamWriter(logPath, append: false);
                writer.WriteLine($"NIF Converter Log - {DateTime.Now:dd/MM/yyyy HH:mm:ss}");
                writer.WriteLine("============================================================");
                writer.WriteLine();
                writer.WriteLine($"Files processed: {fileCount}");
                writer.WriteLine($"Shapes total:    {total}");
                writer.WriteLine();
                writer.WriteLine("Shape categorization");
                writer.WriteLine("----------------------------------------");
                writer.WriteLine($"  Lighting + inline TextureSet:   {lightingInline}  ({Pct(lightingInline, total)})");
                writer.WriteLine($"  Lighting + BGSM resolved:       {lightingBgsmResolved}  ({Pct(lightingBgsmResolved, total)})");
                writer.WriteLine($"  Lighting + BGSM unresolved:     {lightingBgsmUnresolved}  ({Pct(lightingBgsmUnresolved, total)})");
                writer.WriteLine($"  Lighting + empty:               {lightingEmpty}  ({Pct(lightingEmpty, total)})");
                writer.WriteLine($"  Effect shader (total):          {effect}  ({Pct(effect, total)})");
                writer.WriteLine($"    -> BGEM resolved:             {effectBgemResolved}  ({Pct(effectBgemResolved, total)})");
                writer.WriteLine($"    -> unresolved:                {effectUnresolved}  ({Pct(effectUnresolved, total)})");
                writer.WriteLine($"  Water shader (-> FX):           {water}  ({Pct(water, total)})");
                writer.WriteLine($"  Other shader:                   {other}  ({Pct(other, total)})");
                writer.WriteLine($"  No shader ref:                  {noShader}  ({Pct(noShader, total)})");
                writer.WriteLine($"  Material census entries:        {MaterialCensus.Count}");
                writer.WriteLine($"  Texture paths repaired:         {_texPathsRepaired}");
                writer.WriteLine($"  Tangent vertices:               {_tangentVertices}");
                writer.WriteLine($"  WARNING UV-gradient tangents:   {_tangentUvGradientVertices}");
                writer.WriteLine($"  WARNING degenerate tangents:    {_tangentDegenerateVertices}");
                writer.WriteLine();
                writer.WriteLine("Animation and skin handling");
                writer.WriteLine("----------------------------------------");
                foreach (var line in NifAnimationStats.AnimationSummaryLines())
                    writer.WriteLine($"  {line}");
                writer.WriteLine();
                writer.WriteLine("Effects and particles");
                writer.WriteLine("----------------------------------------");
                foreach (var line in NifAnimationStats.EffectSummaryLines())
                    writer.WriteLine($"  {line}");
                writer.WriteLine();
                writer.WriteLine($"Missing Textures ({_missingTextures.Count}):");
                writer.WriteLine("----------------------------------------");
                foreach (var key in _missingTextures.Keys.OrderBy(k => k))
                    writer.WriteLine(key);
                writer.WriteLine();
                writer.WriteLine();
                writer.WriteLine($"Unsupported Shader Types ({_unsupportedShaders.Count}):");
                writer.WriteLine("----------------------------------------");
                foreach (var key in _unsupportedShaders.Keys.OrderBy(k => k))
                    writer.WriteLine(key);
                writer.WriteLine();
                writer.WriteLine();
                writer.WriteLine($"Errors ({_errors.Count}):");
                writer.WriteLine("----------------------------------------");
                foreach (var err in _errors)
                    writer.WriteLine(err);
                writer.WriteLine();
                writer.WriteLine($"Warnings ({_warnings.Count}):");
                writer.WriteLine("----------------------------------------");
                foreach (var warning in _warnings)
                    writer.WriteLine(warning);
            }
            catch (Exception ex)
            {
                Console.WriteLine($"Failed to write nif_converter.log: {ex.Message}");
            }
        }

        private static string Pct(int part, int total)
        {
            if (total <= 0) return "0.00%";
            return $"{(100.0 * part / total):F2}%";
        }

        public static void ConvertToGltf(string source, string destination)
        {
            var model = ConvertNifToGltf(source);
            if (model != null)
            {
                Directory.CreateDirectory(Path.GetDirectoryName(destination)!);
                model.SaveGLB(destination);
            }
        }

        /// <summary>The effect shader family's descriptor set; effect shapes stay hidden legacy FX (and are counted) while it is unset.</summary>
        internal static IEffectShaderCatalog? EffectCatalog { get; set; }

        // Tolerated per-shape property-read failures, aggregated per site and reported once per batch.
        internal static ReadFailureTally ReadFailures { get; private set; } = new();

        // Conversion of one NIF stays on one thread, so the failing NIF's path is available to the shape helpers.
        [ThreadStatic] private static string? _currentNifPath;
        private static string CurrentNifPath => _currentNifPath ?? "<unknown nif>";

        public const string EmitterSourcePointsCounter = "particle emitter source shapes without triangles exported as points";

        private static void WarnCurrentFile(string message) => _warnings.Add($"{CurrentNifPath}: {message}");

        private static ModelRoot? ConvertNifToGltf(string source)
        {
            _currentNifPath = source;
            if (new FileInfo(source).Length == 0)
                return null;

            var nif = new NifFile();
            try
            {
                using var stream = IoRetry.Run(() => new FileStream(source, FileMode.Open, FileAccess.Read, FileShare.Read), IoRetry.IsFileBusy, IoRetry.DefaultDelays);
                int loadResult = nif.Load(stream);
                if (loadResult != 0)
                {
                    _errors.Add($"{source}: NIF load failed with code {loadResult}");
                    return null;
                }
            }
            catch (Exception ex)
            {
                _errors.Add($"{source}: NIF load threw {ex.GetType().Name}: {ex.Message}");
                return null;
            }

            void ReportWarning(string message) => _warnings.Add($"{source}: {message}");
            var clips = NifControllerExtractor.Extract(nif, ReportWarning);
            var animatedNodeNames = clips.SelectMany(c => c.Tracks).Select(t => t.NodeName).ToHashSet(StringComparer.Ordinal);

            var scene = new SceneBuilder();
            var nodeMap = new Dictionary<int, NodeBuilder>();
            var isChild = new HashSet<int>();
            var processedShapes = new HashSet<int>();
            var materialCache = new Dictionary<string, MaterialBuilder>();
            var emitterSources = EmitterSourceShapes(nif);

            // 1. Create Nodes
            for (int i = 0; i < nif.Blocks.Count; i++)
            {
                if (nif.Blocks[i] is NiNode niNode)
                {
                    var node = new NodeBuilder(NifNodeNames.Of(nif, i));
                    node.LocalTransform = GetTransform(niNode);
                    nodeMap[i] = node;
                }
            }

            // 2. Build Hierarchy
            for (int i = 0; i < nif.Blocks.Count; i++)
            {
                if (nif.Blocks[i] is NiNode niNode && nodeMap.TryGetValue(i, out var parentNode))
                {
                    if (niNode.Children.Indices != null)
                    {
                        foreach (var childIndex in niNode.Children.Indices)
                        {
                            if (nodeMap.TryGetValue(childIndex, out var childNode))
                            {
                                parentNode.AddNode(childNode);
                                isChild.Add(childIndex);
                            }
                        }
                    }
                }
            }

            // Add root nodes to scene. Strip any baked transform on root NiNodes:
            // Bethesda often wraps the model in a root NiNode with a 180° Y
            // rotation (Max/Maya axis-convention compensation). The CK/game
            // engine ignores the root NiNode's transform at REFR render time
            // — REFR rotation is applied directly to the design-frame vertices.
            // If we preserve the root's 180° here, every placement ends up
            // 180° off from what the CK shows. Children of the root keep their
            // transforms (those are real per-shape offsets).
            foreach (var kvp in nodeMap)
            {
                if (!isChild.Contains(kvp.Key))
                {
                    kvp.Value.LocalTransform = Matrix4x4.Identity;
                    scene.AddNode(kvp.Value);
                }
            }

            // 3. Process Meshes (Children of NiNodes)
            for (int i = 0; i < nif.Blocks.Count; i++)
            {
                if (nif.Blocks[i] is NiNode niNode && nodeMap.TryGetValue(i, out var parentNode))
                {
                    if (niNode.Children.Indices != null)
                    {
                        foreach (var childIndex in niNode.Children.Indices)
                        {
                            if (childIndex < 0) continue;
                            var block = nif.Blocks[childIndex];
                            if (block is BSTriShape shape)
                            {
                                // BSMeshLODTriShape is FO4's primary render shape for statics
                                // with embedded LODs; it is NOT a separate distant-LOD overlay.
                                // Process it the same as BSTriShape — its full vertex + triangle
                                // arrays are the real geometry.
                                ProcessShape(shape, nif, scene, nodeMap, parentNode, materialCache, source, animatedNodeNames, emitterSources);
                                processedShapes.Add(childIndex);
                            }
                        }
                    }
                }
            }

            // 4. Process Root Meshes (Not children of any NiNode). Same
            // rationale as step 3: root-level nodes get identity transform so
            // REFR rotation acts on the design-frame vertices directly.
            for (int i = 0; i < nif.Blocks.Count; i++)
            {
                if (nif.Blocks[i] is BSTriShape shape && !processedShapes.Contains(i))
                {
                    var name = ShapeName(shape);
                    if (string.IsNullOrEmpty(name)) name = $"Mesh_{i}";
                    var node = new NodeBuilder(name);
                    // intentionally skip GetTransform for root-level shapes
                    scene.AddNode(node);
                    ProcessShape(shape, nif, scene, nodeMap, node, materialCache, source, animatedNodeNames, emitterSources);
                }
            }

            var effectNodes = new NifEffectNodes(nif, scene, nodeMap);
            effectNodes.Attach(clips, EffectCatalog, raw => CleanTexturePathForDeploy(raw, isDiffuse: false), ReportWarning);
            foreach (var (block, node) in nodeMap)
                NifNodeExtras.ApplyObjectState((NiAVObject)nif.Blocks[block], node);
            if (nodeMap.Keys.Where(block => !isChild.Contains(block)).DefaultIfEmpty(-1).Min() is var rootBlock and >= 0)
                NifNodeExtras.ApplyConnectPoints(nif, nodeMap, rootBlock, ReportWarning);

            var model = scene.ToGltf2();
            effectNodes.ResolveMaterials(model);
            NifEffectNodes.VerifyReferences(model);
            NifSkinConverter.NameSkinnedNodes(model);
            GltfAnimationWriter.Write(model, clips, ReportWarning);
            GltfNodeNames.EnsureUnique(model);
            return model;
        }

        private static void ProcessShape(BSTriShape shape, NifFile nif, SceneBuilder scene,
            Dictionary<int, NodeBuilder> nodeMap, NodeBuilder parentNode,
            Dictionary<string, MaterialBuilder> materialCache, string source, ISet<string> animatedNodeNames, HashSet<BSTriShape> emitterSources)
        {
            // CK-only marker geometry (crow-perch wire lines, editor arrows, bounds
            // visualizations). The game engine hides shapes with this name at runtime;
            // converting them produces untextured meshes — most visibly the magenta
            // "powerline wires" strung across downtown (CrowMarkerLine*.nif).
            // Contains, not StartsWith: 46 plant/tree NIFs name theirs
            // "VisibilityEditorMarker:0" (the floating boxes on bushes).
            if (ShapeName(shape).Contains("EditorMarker", StringComparison.OrdinalIgnoreCase))
                return;

            MaterialBuilder material;
            try
            {
                material = GetMaterial(shape, nif, materialCache);
            }
            catch (WaterShaderUnreadableException ex)
            {
                _errors.Add($"{source}: water shape '{ShapeName(shape)}' not converted: {ex.Message}");
                return;
            }
            if (!HasGeometry(shape))
            {
                if (emitterSources.Contains(shape) && shape.VertexPositions is { Count: > 0 })
                    AddEmitterSourcePoints(shape, scene, parentNode, material);
                return;
            }

            var shapeName = ShapeName(shape);
            if (string.IsNullOrEmpty(shapeName)) shapeName = "shape";
            var vertexData = ReadVertexData(shape);

            if (shape.IsSkinned && shape.SkinInstanceRef.Index >= 0)
            {
                try
                {
                    NifSkinConverter.Add(shape, shapeName, vertexData, material, nif, scene, nodeMap, animatedNodeNames);
                }
                catch (NifSkinException ex)
                {
                    NifAnimationStats.Add(NifAnimationStats.SkinsFailed);
                    _warnings.Add($"{source}: skinned shape '{shapeName}' dropped: {ex.Message}");
                }
                return;
            }

            var mesh = NifMeshBuilder.Rigid("mesh", vertexData, shape.Triangles, material);
            var shapeNode = new NodeBuilder(shapeName);
            shapeNode.LocalTransform = GetTransform(shape);
            parentNode.AddNode(shapeNode);
            NifNodeExtras.ApplyObjectState(shape, shapeNode);
            scene.AddRigidMesh(mesh, shapeNode);
        }

        // A mesh emitter that spawns from vertices may reference a shape with vertices but no triangles. The particle extras name
        // that shape, so it is exported as a node carrying a point primitive; the engine samples its vertices and hides it.
        private static void AddEmitterSourcePoints(BSTriShape shape, SceneBuilder scene, NodeBuilder parentNode, MaterialBuilder material)
        {
            var name = ShapeName(shape);
            var points = NifMeshBuilder.Points("mesh", shape.VertexPositions.Select(GameCoordinates.ToGltfPosition), material);
            var node = new NodeBuilder(string.IsNullOrEmpty(name) ? "shape" : name) { LocalTransform = GetTransform(shape) };
            parentNode.AddNode(node);
            NifNodeExtras.ApplyObjectState(shape, node);
            scene.AddRigidMesh(points, node);
            NifAnimationStats.Add(EmitterSourcePointsCounter);
        }

        private static HashSet<BSTriShape> EmitterSourceShapes(NifFile nif)
        {
            var sources = new HashSet<BSTriShape>(ReferenceEqualityComparer.Instance);
            foreach (var emitter in nif.Blocks.OfType<NiPSysMeshEmitter>())
            {
                foreach (int index in emitter.EmitterMeshes.Indices.Where(i => i >= 0 && i < nif.Blocks.Count))
                {
                    if (nif.Blocks[index] is BSTriShape shape)
                        sources.Add(shape);
                }
            }
            return sources;
        }

        private static bool HasGeometry(BSTriShape shape)
            => shape.VertexPositions is { Count: > 0 } && shape.Triangles is { Count: > 0 };

        private static ShapeVertexData ReadVertexData(BSTriShape shape)
        {
            var (positions, normals, texCoords) = ToGltfVertexData(shape, shape.VertexPositions);
            var tangents = BuildTangents(shape, positions, normals, texCoords);
            return new ShapeVertexData(positions, normals, tangents, texCoords, ReadVertexColors(shape, positions.Length));
        }

        // Grass models carry their blade tint (RGB) and sway weight (A) here. Vanilla draws shapes
        // without a colour stream with instancing shaders that neither tint nor sway, which is
        // exactly white with zero alpha.
        internal static readonly Vector4 NoVertexColors = new(1f, 1f, 1f, 0f);

        private static Vector4[] ReadVertexColors(BSTriShape shape, int vertexCount)
        {
            var colors = new Vector4[vertexCount];
            var source = shape.HasVertexColors ? shape.VertexColors : null;
            for (int i = 0; i < vertexCount; i++)
            {
                colors[i] = source != null && i < source.Count
                    ? Vector4.Clamp(new Vector4(source[i].R, source[i].G, source[i].B, source[i].A), Vector4.Zero, Vector4.One)
                    : NoVertexColors;
            }
            return colors;
        }

        // NiAVObject.Name is a NiStringRef: the string content is in .String;
        // ToString() yields the type name ("NiflySharp.NiStringRef").
        private static string ShapeName(BSTriShape shape)
        {
            return ReadFailures.Read("shape name", CurrentNifPath, () => shape.Name?.String ?? "", "");
        }

        private static Matrix4x4 GetTransform(NiAVObject node)
            => GameCoordinates.ToGltfTransform(node.Rotation, node.Translation, node.Scale);

        private static MaterialBuilder GetMaterial(BSTriShape shape, NifFile nif,
            Dictionary<string, MaterialBuilder> materialCache)
        {
            Interlocked.Increment(ref _shapesTotal);

            string? diffusePath = null;
            string? normalPath = null;
            string alphaToken = "";
            bool twoSided = false;
            bool isEffectShader = false;
            WaterShaderParams? waterParams = null;
            // Census def of the BGSM/BGEM that resolved, when one did. Carries the
            // source-material identity (Key -> "src:" token, used by the engine to
            // match MSWP material-swap substitutions to this primitive) and the
            // palette combo for baked-variant diffuse naming.
            MaterialDef? sourceDef = null;
            var lighting = LightingParams.None;
            JsonObject? effectExtras = null;
            BSLightingShaderProperty? refractionCandidate = null;
            EffectParams? effect = null;
            EnvironmentParams? env = null;

            // NiAlphaProperty on the shape is the authoritative source for inline-texture
            // shapes (trees, foliage, rebar) that don't use a BGSM. Check it first; if present
            // it wins over any BGSM-derived token below.
            string shapeAlphaToken = GetAlphaTokenFromShape(shape, nif);

            if (shape.ShaderPropertyRef.Index < 0)
            {
                Interlocked.Increment(ref _shapesNoShaderRef);
            }
            else
            {
                var shaderBlock = nif.Blocks[shape.ShaderPropertyRef.Index];

                if (shaderBlock is BSLightingShaderProperty lightingShader)
                {
                    refractionCandidate = lightingShader;
                    BSShaderTextureSet? inlineTextureSet = null;
                    if (lightingShader.TextureSetRef.Index >= 0 &&
                        nif.Blocks[lightingShader.TextureSetRef.Index] is BSShaderTextureSet texSet)
                    {
                        inlineTextureSet = texSet;
                        if (texSet.Textures != null && texSet.Textures.Count > 0)
                        {
                            if (texSet.Textures.Count > 0)
                                diffusePath = texSet.Textures[0]?.Content;

                            if (texSet.Textures.Count > 1)
                                normalPath = texSet.Textures[1]?.Content;
                        }
                    }

                    bool inlineTextures = !string.IsNullOrEmpty(diffusePath) || !string.IsNullOrEmpty(normalPath);
                    string? bgsmRef = GetBgsmReference(lightingShader);

                    // The BGSM is authoritative when present: the game engine replaces the
                    // NIF's inline TextureSet with the material file's textures, and many
                    // vanilla NIFs carry stale inline paths (placeholder gray.dds, .tga
                    // references, bare filenames). Inline textures are the fallback for
                    // shapes without a resolvable material (trees, foliage, rebar).
                    if (bgsmRef != null && MaterialCensus.TryResolve(bgsmRef, out var bgsmDef))
                    {
                        lighting = bgsmDef.Lighting;
                        diffusePath = bgsmDef.Diffuse;
                        normalPath = bgsmDef.Normal;
                        alphaToken = bgsmDef.AlphaToken;
                        twoSided = bgsmDef.TwoSided;
                        sourceDef = bgsmDef;
                        env = bgsmDef.Env;
                        Interlocked.Increment(ref _shapesLightingBgsmResolved);
                        if (_bgsmRefSamplesResolved.Count < _bgsmSampleCap)
                            _bgsmRefSamplesResolved.TryAdd(bgsmRef, 0);
                    }
                    else if (inlineTextures)
                    {
                        twoSided = lightingShader.DoubleSided;
                        lighting = LightingParams.FromInline(lightingShader, inlineTextureSet);
                        Interlocked.Increment(ref _shapesLightingInlineTextures);
                    }
                    else if (bgsmRef != null)
                    {
                        Interlocked.Increment(ref _shapesLightingBgsmReference);
                        if (_bgsmRefSamplesUnresolved.Count < _bgsmSampleCap)
                            _bgsmRefSamplesUnresolved.TryAdd(bgsmRef, 0);
                    }
                    else
                    {
                        Interlocked.Increment(ref _shapesLightingEmpty);
                    }
                }
                else if (shaderBlock is BSEffectShaderProperty effectShader)
                {
                    Interlocked.Increment(ref _shapesEffectShader);
                    // Tagged "|fx" in the material name. The shader block is read once; the effect extras (effect
                    // stage) and the bm/bc/bs/fo/li/pal/env tokens (object-shader path) both derive from it or from
                    // the referenced BGEM.
                    isEffectShader = true;
                    var shaderData = EffectShapeConversion.TryRead(shape, effectShader, WarnCurrentFile);
                    var alphaState = EffectShapeConversion.ReadAlpha(shape, nif);
                    string? bgemRef = GetEffectBgemReference(effectShader);
                    bool resolved = false;
                    if (bgemRef != null && MaterialCensus.TryResolve(bgemRef, out var bgemDef))
                    {
                        diffusePath = bgemDef.Diffuse;
                        normalPath = bgemDef.Normal;
                        alphaToken = bgemDef.AlphaToken;
                        twoSided = bgemDef.TwoSided;
                        sourceDef = bgemDef;
                        effect = bgemDef.Effect is { } bgemMaterial ? EffectParams.FromMaterial(bgemMaterial) : null;
                        env = bgemDef.Env;
                        resolved = true;
                    }
                    else if (shaderData != null)
                    {
                        effect = EffectParams.FromShader(shaderData, alphaState);
                        env = ResolveInlineEnvironment(shaderData);
                        float inlineAlpha = shaderData.BaseColor.W;
                        if (shapeAlphaToken == "blend" && MathF.Abs(inlineAlpha - 1f) > MaterialCensus.AlphaScaleTolerance)
                            alphaToken = $"blend|{inlineAlpha.ToString("0.####", System.Globalization.CultureInfo.InvariantCulture)}";
                        twoSided = effectShader.DoubleSided;
                        if (!string.IsNullOrEmpty(shaderData.SourceTexture) || !string.IsNullOrEmpty(shaderData.NormalTexture))
                        {
                            diffusePath = shaderData.SourceTexture;
                            normalPath = shaderData.NormalTexture;
                            resolved = true;
                        }
                    }

                    effectExtras = EffectShapeConversion.TryBuild(shape, nif, shape.ShaderPropertyRef.Index, effectShader, shaderData, sourceDef,
                        EffectCatalog, raw => CleanTexturePathForDeploy(raw, isDiffuse: false), WarnCurrentFile);

                    if (resolved)
                    {
                        Interlocked.Increment(ref _shapesEffectBgemResolved);
                    }
                    else
                    {
                        Interlocked.Increment(ref _shapesEffectUnresolved);
                        if (bgemRef != null && _bgemRefSamplesUnresolved.Count < _bgsmSampleCap)
                            _bgemRefSamplesUnresolved.TryAdd(bgemRef, 0);
                    }
                }
                else if (shaderBlock is BSWaterShaderProperty waterShader)
                {
                    Interlocked.Increment(ref _shapesWaterShader);
                    waterParams = WaterShaderParams.FromShader(waterShader);
                    // The shader's own fields travel in the |water: token and the engine draws water with
                    // its own renderer, so |fx no longer hides it. The borrowed water diffuse
                    // (_waterFxDiffuse) is only there so the shape keeps a texture.
                    isEffectShader = true;
                    diffusePath = _waterFxDiffuse;
                    normalPath = _waterFxNormal;
                    alphaToken = "blend";
                }
                else
                {
                    var shaderType = shaderBlock?.GetType().Name ?? "Unknown";
                    _unsupportedShaders.TryAdd(shaderType, 0);
                    Interlocked.Increment(ref _shapesOtherShader);
                }
            }

            // Convert paths to clean format
            string cleanDiffuse = diffusePath != null ? ConvertTexturePath(diffusePath) : "";
            string cleanNormal = normalPath != null ? ConvertTexturePath(normalPath) : "";
            cleanDiffuse = StripRemapToken(cleanDiffuse);

            // Grayscale-to-palette materials reference the texture the palette
            // baker writes for this material's (LUT, row) combo. The per-diffuse
            // winner combo keeps the original filename (baked in place, exactly
            // the pre-variant behavior); every other combo (sportscar paint
            // colors etc.) lives in a ".pal-xxxxxxxx" variant file. Variant
            // names skip the repair logic — those files exist only post-bake.
            if (sourceDef is { IsPalette: true })
            {
                string texKey = MaterialCensus.NormalizeTexPath(sourceDef.Diffuse);
                string baked = MaterialCensus.BakedDiffuseName(texKey, sourceDef.Lut, sourceDef.Row);
                if (baked != texKey)
                    cleanDiffuse = baked;
            }

            // BGSM/BGEM is the semantic source of truth for the alpha mode (it encodes the
            // artist's intent: blend vs scissor). NiAlphaProperty is a NIF render-state block
            // that is used as a fallback only when there is no material file token — it is the
            // only alpha source for inline-textured shapes (trees, foliage, rebar) that don't
            // reference a BGSM. Keeping NiAlphaProperty subordinate prevents its AlphaTest
            // flag from overriding a BGSM that correctly requests blend mode (glass, effects).
            string finalAlphaToken = !string.IsNullOrEmpty(alphaToken) ? alphaToken : shapeAlphaToken;

            JsonObject? refractionExtras = refractionCandidate == null ? null
                : RefractionShapeConversion.TryBuild(shape, nif, shape.ShaderPropertyRef.Index, refractionCandidate, cleanDiffuse, cleanNormal, WarnCurrentFile);
            var materialExtras = effectExtras ?? refractionExtras;

            // Material name grammar and its examples live in MaterialNameCodec.
            string cacheKey = MaterialNameCodec.Encode(new MaterialNameParts(
                Diffuse: cleanDiffuse,
                Normal: cleanNormal,
                AlphaToken: finalAlphaToken,
                Effect: isEffectShader,
                Decal: sourceDef is { Decal: true },
                Lighting: MaterialNameCodec.CleanLighting(lighting),
                TwoSided: twoSided,
                SourceKey: sourceDef?.Key,
                Water: waterParams,
                Refraction: refractionExtras != null,
                ExtrasKey: materialExtras == null ? null : GltfMaterialExtras.KeyOf(materialExtras),
                // The effect and refraction stages read their own extras; the tokens serve the object-shader path.
                EffectFields: materialExtras == null ? effect?.WithTexturePaths(MaterialNameCodec.CleanPath) : null,
                Env: materialExtras == null ? env?.WithTexturePaths(MaterialNameCodec.CleanPath) : null));

            string materialCacheKey = GltfMaterialExtras.CacheKey(cacheKey, materialExtras);
            if (materialCache.TryGetValue(materialCacheKey, out var cachedMaterial))
                return cachedMaterial;

            // Alpha mode is encoded in the cacheKey/material name for the Godot side
            // to parse; we intentionally do NOT set WithAlpha() on the glTF itself,
            // because Godot recreates the material from scratch using the name and
            // setting glTF-level alpha mode just churns the import result.
            var material = new MaterialBuilder(cacheKey)
                .WithMetallicRoughnessShader()
                .WithMetallicRoughness(0f, 1f)
                .WithBaseColor(new Vector4(1, 1, 1, 1));
            if (materialExtras != null)
                GltfMaterialExtras.Attach(material, refractionExtras != null ? GltfMaterialExtras.RefractionKind : GltfMaterialExtras.EffectKind, materialExtras);

            materialCache[materialCacheKey] = material;
            return material;
        }

        // Heuristic: on FO4 BSLightingShaderProperty, the Name field carries the BGSM rootMaterialName
        // (e.g. "Materials\\Architecture\\...\\Foo.bgsm"). Anything non-empty here means this shape
        // depends on an external material file, so the inline TextureSet being empty is expected.
        private static string? GetBgsmReference(BSLightingShaderProperty shader)
        {
            // shader.Name is a NiStringRef — the actual string lives in .String.
            // (NiString has .Content; NiStringRef is a different type.)
            var name = ReadFailures.Read("lighting shader BGSM reference", CurrentNifPath, () => shader.Name?.String, (string?)null);
            return string.IsNullOrWhiteSpace(name) ? null : name;
        }

        // Same pattern for BSEffectShaderProperty, except the referenced file is a .bgem
        // (effect-shader material) rather than a .bgsm.
        internal static string? GetEffectBgemReference(BSEffectShaderProperty shader)
        {
            var name = ReadFailures.Read("effect shader BGEM reference", CurrentNifPath, () => shader.Name?.String, (string?)null);
            return string.IsNullOrWhiteSpace(name) ? null : name;
        }

        // FO4 "remap" textures (e.g. residentialsidingremap01_d) are layered-material
        // palette textures that the FO4 engine tints at runtime. Without that shader they
        // appear purple. Strip the standalone "remap" token from the filename (only when
        // NOT followed by a letter, so "remapped"/"remapping" are left untouched) to fall
        // back to the base texture (e.g. residentialsiding01_d).
        private static string StripRemapToken(string cleanDiffuse)
        {
            if (!cleanDiffuse.Contains("remap"))
                return cleanDiffuse;

            int lastSlash = cleanDiffuse.LastIndexOf('/');
            string dir = lastSlash >= 0 ? cleanDiffuse[..(lastSlash + 1)] : "";
            string filename = lastSlash >= 0 ? cleanDiffuse[(lastSlash + 1)..] : cleanDiffuse;
            int ri = filename.IndexOf("remap", StringComparison.Ordinal);
            if (ri >= 0)
            {
                int after = ri + "remap".Length;
                bool standalone = after >= filename.Length || !char.IsLetter(filename[after]);
                if (standalone)
                    filename = filename.Remove(ri, "remap".Length);
            }
            // The synthesized base-texture name may itself need repair
            // (idempotent for paths that already exist).
            return RepairTexturePath(dir + filename);
        }

        // ---- Facade for the ESM loader's material-swap resolution ----------
        // MaterialSwapProcessor pre-resolves MSWP substitutions into final
        // deployed texture paths at install time; those paths must be cleaned
        // byte-identically to the ones this converter writes into GLB material
        // names, or the engine's texture cache keys won't line up.

        /// <summary>Builds the texture index if this process hasn't run the
        /// converter (e.g. the esm-only dev subcommand).</summary>
        public static void EnsureTexturePathContext(string stagingRoot)
        {
            if (_textureIndex.Count == 0)
                BuildTextureIndex(Path.Combine(stagingRoot, "textures"));
        }

        /// <summary>Raw material-file texture path -> deployed path, with the
        /// same normalization + repair + remap-strip GLB names get.</summary>
        public static string CleanTexturePathForDeploy(string rawPath, bool isDiffuse)
        {
            if (string.IsNullOrEmpty(rawPath)) return "";
            string clean = ConvertTexturePath(rawPath);
            return isDiffuse ? StripRemapToken(clean) : clean;
        }

        // NiAlphaProperty fallback: used only when no BGSM/BGEM token is available.
        // When both AlphaBlend and AlphaTest flags are set, prefer blend — that matches the
        // FO4 convention where glass sets both flags but is intended to be semi-transparent.
        private static string GetAlphaTokenFromShape(BSTriShape shape, NifFile nif)
        {
            return ReadFailures.Read("alpha property", CurrentNifPath, () =>
            {
                if (GetAlphaProperty(shape, nif) is not { } alphaProp) return "";

                if (alphaProp.Flags.AlphaBlend)
                    return "blend";
                if (alphaProp.Flags.AlphaTest)
                    return $"test:{alphaProp.Threshold}";
                return "";
            }, "");
        }

        private static NiAlphaProperty? GetAlphaProperty(BSTriShape shape, NifFile nif)
        {
            return ReadFailures.Read("alpha property block", CurrentNifPath, () =>
            {
                if (!shape.HasAlphaProperty) return null;
                int idx = shape.AlphaPropertyRef.Index;
                if (idx < 0 || idx >= nif.Blocks.Count) return null;
                return nif.Blocks[idx] as NiAlphaProperty;
            }, (NiAlphaProperty?)null);
        }

        private static EnvironmentParams? ResolveInlineEnvironment(EffectShaderData shader)
        {
            var env = EnvironmentParams.Resolve(EffectShaderFlags.Has(shader.Flags1, EffectShaderFlags.EnvironmentMapping), shader.EnvTexture,
                shader.EnvMaskTexture, shader.EnvironmentMapScale, MaterialCensus.EnvTextureExists, out bool cubeMissing);
            if (cubeMissing) Interlocked.Increment(ref _shapesInlineEnvCubeMissing);
            return env;
        }

        private static string ConvertTexturePath(string ddsPath)
        {
            var clean = ddsPath.Replace("/", "\\").ToLowerInvariant();

            if (clean.StartsWith("textures\\", StringComparison.Ordinal))
                clean = clean.Substring(9);

            clean = clean.Replace("\\", "/");

            return RepairTexturePath(clean);
        }

        private static void BuildTextureIndex(string texturesRoot)
        {
            if (_textureIndexPreloaded) return; // pipeline pre-seeded from BA2 name tables

            _textureIndex = new HashSet<string>();
            _texturesByName = new Dictionary<string, List<string>>();
            if (!Directory.Exists(texturesRoot))
            {
                Console.WriteLine($"Texture index: {texturesRoot} not found; texture path repair disabled.");
                return;
            }
            foreach (var file in Directory.EnumerateFiles(texturesRoot, "*.dds", SearchOption.AllDirectories))
            {
                string rel = Path.GetRelativePath(texturesRoot, file).Replace('\\', '/').ToLowerInvariant();
                _textureIndex.Add(rel);
                string name = Path.GetFileName(rel);
                if (!_texturesByName.TryGetValue(name, out var list))
                    _texturesByName[name] = list = new List<string>();
                list.Add(rel);
            }
            Console.WriteLine($"Texture index: {_textureIndex.Count} files.");
        }

        private static bool _textureIndexPreloaded;

        internal static bool TextureIndexAvailable => _textureIndex.Count > 0;

        /// <summary>True when the path, after the same repair texture references get, is in the texture index.</summary>
        internal static bool TextureResolves(string normalizedPath) =>
            TextureIndexContains(RepairTexturePath(normalizedPath));

        /// <summary>
        /// Seeds the texture-existence index from relative texture paths (as
        /// stored in BA2 name tables, e.g. "textures\foo\bar_d.dds") without
        /// requiring the textures to exist on disk yet. Lets the pipelined
        /// installer start NIF conversion while texture archives are still
        /// extracting. Paths are normalized exactly like BuildTextureIndex:
        /// lowercase, forward slashes, leading "textures/" stripped.
        /// </summary>
        public static void PreloadTextureIndex(IEnumerable<string> archiveRelativePaths)
        {
            _textureIndex = new HashSet<string>();
            _texturesByName = new Dictionary<string, List<string>>();
            foreach (var p in archiveRelativePaths)
            {
                string rel = p.Replace('\\', '/').ToLowerInvariant();
                if (rel.StartsWith("textures/", StringComparison.Ordinal)) rel = rel.Substring(9);
                if (!rel.EndsWith(".dds", StringComparison.Ordinal)) continue;
                if (!_textureIndex.Add(rel)) continue;
                string name = Path.GetFileName(rel);
                if (!_texturesByName.TryGetValue(name, out var list))
                    _texturesByName[name] = list = new List<string>();
                list.Add(rel);
            }
            _textureIndexPreloaded = true;
            Console.WriteLine($"Texture index (from archive name tables): {_textureIndex.Count} files.");
        }

        /// <summary>Existence check against the texture index (NormalizeTexPath
        /// form: lowercase, forward slashes, no leading "textures/"). Used by
        /// MaterialCensus when textures aren't staged on disk.</summary>
        public static bool TextureIndexContains(string relPath)
        {
            if (string.IsNullOrEmpty(relPath)) return false;
            string rel = relPath.Replace('\\', '/').ToLowerInvariant();
            if (rel.StartsWith("textures/", StringComparison.Ordinal)) rel = rel.Substring(9);
            return _textureIndex.Contains(rel);
        }

        // Vanilla NIF/BGSM texture references include paths that don't exist on disk
        // in known-fixable ways (.tga extensions for textures shipped as .dds, bare
        // filenames without their directory, LOD textures missing the lod/ prefix,
        // omitted or doubled _d suffixes). Repair them against the extracted-texture
        // index; unrepairable paths are returned unchanged and logged so the engine's
        // fallback (and nif_converter.log) make them visible.
        private static string RepairTexturePath(string clean)
        {
            if (clean.Length == 0 || _textureIndex.Count == 0 || _textureIndex.Contains(clean))
                return clean;

            static string Found(string p) { Interlocked.Increment(ref _texPathsRepaired); return p; }

            if (_placeholderTextures.TryGetValue(clean, out var substitute) && _textureIndex.Contains(substitute))
                return Found(substitute);

            string candidate = Path.ChangeExtension(clean, ".dds");
            if (_textureIndex.Contains(candidate))
                return Found(candidate);

            if (!candidate.Contains('/') &&
                _texturesByName.TryGetValue(candidate, out var byName) && byName.Count == 1)
                return Found(byName[0]);

            if (_textureIndex.Contains("lod/" + candidate))
                return Found("lod/" + candidate);

            string stem = candidate[..^4];
            if (_textureIndex.Contains(stem + "_d.dds"))
                return Found(stem + "_d.dds");

            // "foo_d_d.dds" / "foo_d_n.dds" -> "foo_d.dds" / "foo_n.dds"
            if (stem.Length > 4 && stem[^4..^2] == "_d" && stem[^2] == '_')
            {
                string collapsed = stem[..^4] + stem[^2..] + ".dds";
                if (_textureIndex.Contains(collapsed))
                    return Found(collapsed);
            }

            _missingTextures.TryAdd(clean, 0);
            return clean;
        }

        private static (Vector3[] Positions, Vector3[] Normals, Vector2[] TexCoords) ToGltfVertexData(
            BSTriShape shape, List<Vector3> verts)
        {
            var norms = shape.Normals;
            var uvs = shape.UVs;
            var positions = new Vector3[verts.Count];
            var normals = new Vector3[verts.Count];
            var texCoords = new Vector2[verts.Count];

            for (int i = 0; i < verts.Count; i++)
            {
                var v = verts[i];
                var n = (norms != null && i < norms.Count) ? norms[i] : Vector3.UnitY;
                positions[i] = GameCoordinates.ToGltfPosition(v);
                normals[i] = SanitizeNormal(GameCoordinates.ToGltfDirection(n));
                texCoords[i] = (uvs != null && i < uvs.Count) ? new Vector2(uvs[i].U, uvs[i].V) : Vector2.Zero;
            }
            return (positions, normals, texCoords);
        }

        private static Vector4[] BuildTangents(BSTriShape shape, Vector3[] positions, Vector3[] normals, Vector2[] texCoords)
        {
            var tangents = NifTangents.Build(shape, positions, normals, texCoords, out var stats);
            Interlocked.Add(ref _tangentVertices, stats.Vertices);
            Interlocked.Add(ref _tangentUvGradientVertices, stats.UvGradientVertices);
            Interlocked.Add(ref _tangentDegenerateVertices, stats.DegenerateVertices);
            return tangents;
        }

        private static Vector3 SanitizeNormal(Vector3 normal)
        {
            if (float.IsNaN(normal.X) || float.IsNaN(normal.Y) || float.IsNaN(normal.Z) ||
                float.IsInfinity(normal.X) || float.IsInfinity(normal.Y) || float.IsInfinity(normal.Z))
            {
                return Vector3.UnitY;
            }

            if (normal.LengthSquared() < 0.0001f)
            {
                return Vector3.UnitY;
            }

            return Vector3.Normalize(normal);
        }
    }
}
