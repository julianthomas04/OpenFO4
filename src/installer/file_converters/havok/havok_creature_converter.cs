using System.Text.Json;
using AssetConverter.Pipeline;

namespace AssetConverter.Havok
{
    public sealed record HavokCreatureReport(
        string Name, int SkeletonBones, int ClipsOk, int ClipsFailed, int NonAnimationFiles, int Packs, long BytesWritten,
        IReadOnlyList<string> Errors, IReadOnlyList<string> Warnings, int TracksDropped);

    /// <summary>
    /// Converts one <see cref="HavokSkeletonUnit"/> into one *.anims.glb clip pack per immediate Animations subfolder and a
    /// skeleton.anims.json that names the NIF-converted render skeleton the clips bind to by bone name.
    /// </summary>
    public static class HavokCreatureConverter
    {
        public const string AssetsFolder = "CharacterAssets";
        public const string AnimationsFolder = "Animations";
        public const string SkeletonSourceName = "skeleton.hkx";
        public const string RenderSkeletonName = "skeleton.glb";
        public const string LegacySkeletonOutputName = "skeleton.skel.glb";
        public const string IndexOutputName = "skeleton.anims.json";
        public const string PackSuffix = ".anims.glb";
        public const string RootPackName = "root";

        private const int InFlightPerWorker = 2;
        private const string ClipSearchPattern = "*.hkx";
        private static readonly JsonSerializerOptions IndexJsonOptions = new() { WriteIndented = true, PropertyNamingPolicy = JsonNamingPolicy.CamelCase };

        private sealed record ClipSource(string Name, string Path, string Pack);

        private sealed record ClipOutcome(ClipSource Source, IReadOnlyList<HavokClip>? Clips, string? Error, bool NonAnimation, string? Warning = null, int DroppedTracks = 0);

        private sealed record PackIndexEntry(string File, IReadOnlyList<string> Clips);

        private sealed record AnimationIndex(string Skeleton, IReadOnlyList<PackIndexEntry> Packs);

        /// <param name="memory">Admits each pack, from its decode to its save, against the install's memory.</param>
        public static HavokCreatureReport Convert(HavokSkeletonUnit unit, int degreeOfParallelism, MemoryBudget memory, CancellationToken ct)
        {
            DeleteStaleOutputs(unit.OutputDir);
            if (!File.Exists(unit.RenderSkeletonPath))
                throw new HavokRenderSkeletonMissingException(unit.Label, unit.RenderSkeletonPath);

            var sources = EnumerateClipSources(unit.AnimationsDir);
            var skeleton = SelectAnimationSkeleton(unit.SkeletonHkxPath, sources);

            Directory.CreateDirectory(unit.OutputDir);
            var options = new ParallelOptions { MaxDegreeOfParallelism = degreeOfParallelism, CancellationToken = ct };
            var packResults = sources.GroupBy(s => s.Pack, StringComparer.OrdinalIgnoreCase)
                .Select(pack => ConvertPack(skeleton, pack.Key, pack.ToList(), unit.OutputDir, options, memory))
                .ToList();

            var written = packResults.Where(r => r.Index is not null).ToList();
            string indexPath = Path.Combine(unit.OutputDir, IndexOutputName);
            WriteIndex(indexPath, RelativeReference(unit.OutputDir, unit.RenderSkeletonPath), written);

            return new HavokCreatureReport(
                unit.Label, skeleton.BoneNames.Count,
                ClipsOk: packResults.Sum(r => r.ClipsOk), ClipsFailed: packResults.Sum(r => r.ClipsFailed),
                NonAnimationFiles: packResults.Sum(r => r.NonAnimation),
                Packs: written.Count, BytesWritten: written.Sum(r => r.Bytes) + new FileInfo(indexPath).Length,
                Errors: packResults.SelectMany(r => r.Errors).ToList(),
                Warnings: packResults.SelectMany(r => r.Warnings).ToList(),
                TracksDropped: packResults.Sum(r => r.TracksDropped));
        }

        private sealed record PackResult(PackIndexEntry? Index, long Bytes, int ClipsOk, int ClipsFailed, int NonAnimation, IReadOnlyList<string> Errors,
            IReadOnlyList<string> Warnings, int TracksDropped);

        private static List<ClipSource> EnumerateClipSources(string animationsDir)
        {
            if (!Directory.Exists(animationsDir)) return new List<ClipSource>();

            var sources = new List<ClipSource>();
            foreach (string path in Directory.EnumerateFiles(animationsDir, ClipSearchPattern, SearchOption.AllDirectories))
            {
                string relative = Path.GetRelativePath(animationsDir, path).Replace('\\', '/');
                int firstSlash = relative.IndexOf('/');
                string pack = firstSlash < 0 ? RootPackName : relative[..firstSlash];
                if (firstSlash >= 0 && pack.Equals(RootPackName, StringComparison.OrdinalIgnoreCase))
                    throw new HavokPackNameCollisionException(pack);
                sources.Add(new ClipSource(Path.ChangeExtension(relative, null), path, pack));
            }
            return sources.OrderBy(s => s.Name, StringComparer.Ordinal).ToList();
        }

        /// <summary>The skeleton file also holds the ragdoll skeleton; the animation skeleton is the one the clips name as their original skeleton.</summary>
        private static HavokSkeleton SelectAnimationSkeleton(string skeletonPath, IReadOnlyList<ClipSource> sources)
        {
            if (!HavokClipBuilder.TryReadFile(File.ReadAllBytes(skeletonPath), out var file) || !file.HasSkeletons)
                throw new HavokFormatException($"'{skeletonPath}' holds no skeleton");
            if (file.Skeletons.Count == 1)
                return file.Skeletons[0];

            string clipSkeletonName = FindBoundSkeletonName(sources)
                ?? throw new HavokAmbiguousSkeletonException(skeletonPath, $"{file.Skeletons.Count} skeletons and no animation clip names the one to use");
            var matches = file.Skeletons.Where(s => string.Equals(s.Name, clipSkeletonName, StringComparison.Ordinal)).ToList();
            return matches.Count == 1
                ? matches[0]
                : throw new HavokAmbiguousSkeletonException(skeletonPath, $"{matches.Count} of {file.Skeletons.Count} skeletons are named '{clipSkeletonName}'");
        }

        /// <summary>The first readable clip decides; unreadable clips are skipped here and reported by <see cref="DecodeClip"/>.</summary>
        private static string? FindBoundSkeletonName(IReadOnlyList<ClipSource> sources)
        {
            foreach (var source in sources)
            {
                try
                {
                    if (HavokClipBuilder.TryReadFile(File.ReadAllBytes(source.Path), out var file) && file.HasAnimations)
                        return file.Bindings[0].OriginalSkeletonName;
                }
                catch (HavokException)
                {
                    continue;
                }
            }
            return null;
        }

        /// <summary>
        /// Converts one pack. Clips are decoded on at most <c>MaxDegreeOfParallelism</c> threads (the only parallel level) and
        /// appended to the pack in source order as soon as the next clip in line is ready, so a slow clip never idles the other
        /// workers and output stays deterministic. At most <see cref="InFlightPerWorker"/> clips per worker are decoded ahead of
        /// the writer, which bounds memory. A failed pack write is reported on the result and fails only that pack.
        /// </summary>
        private static PackResult ConvertPack(HavokSkeleton skeleton, string pack, List<ClipSource> sources, string assetsOut, ParallelOptions options, MemoryBudget memory)
        {
            using var reservation = memory.Reserve(MemoryEstimates.HavokClipPackBytes(sources.Sum(s => ClipLength(s.Path))), options.CancellationToken);
            var errors = new List<string>();
            var warnings = new List<string>();
            int tracksDropped = 0;
            var clipNames = new List<string>();
            var packWriter = new HavokGltfWriter.ClipPack(skeleton);
            int nonAnimation = 0;
            int decodeFailures = 0;

            foreach (var outcome in DecodeInOrder(skeleton, sources, options))
            {
                if (outcome.NonAnimation) nonAnimation++;
                if (outcome.Warning is not null) warnings.Add(outcome.Warning);
                tracksDropped += outcome.DroppedTracks;
                if (outcome.Error is not null)
                {
                    errors.Add(outcome.Error);
                    decodeFailures++;
                }
                foreach (var clip in outcome.Clips ?? Array.Empty<HavokClip>())
                {
                    packWriter.Add(clip);
                    clipNames.Add(clip.Name);
                }
            }

            if (clipNames.Count == 0)
                return new PackResult(null, 0, 0, decodeFailures, nonAnimation, errors, warnings, tracksDropped);

            string fileName = pack + PackSuffix;
            string path = Path.Combine(assetsOut, fileName);
            try
            {
                packWriter.Save(path);
            }
            catch (Exception e) when (e is IOException or UnauthorizedAccessException)
            {
                errors.Add($"{pack}: pack not written: {e.GetType().Name}: {e.Message}");
                return new PackResult(null, 0, 0, decodeFailures + clipNames.Count, nonAnimation, errors, warnings, tracksDropped);
            }
            return new PackResult(new PackIndexEntry(fileName, clipNames), new FileInfo(path).Length, clipNames.Count, decodeFailures, nonAnimation, errors, warnings, tracksDropped);
        }

        /// <summary>A clip that vanished counts as empty here; reading it later reports the error on its outcome.</summary>
        private static long ClipLength(string path)
        {
            var file = new FileInfo(path);
            return file.Exists ? file.Length : 0;
        }

        /// <summary>Yields the decoded clips in source order while keeping a bounded window of clips decoding ahead of the consumer.</summary>
        private static IEnumerable<ClipOutcome> DecodeInOrder(HavokSkeleton skeleton, IReadOnlyList<ClipSource> sources, ParallelOptions options)
        {
            int workers = Math.Max(1, options.MaxDegreeOfParallelism);
            int window = workers * InFlightPerWorker;
            using var slots = new SemaphoreSlim(workers);
            var pending = new Queue<Task<ClipOutcome>>();
            int next = 0;
            try
            {
                while (next < sources.Count || pending.Count > 0)
                {
                    while (next < sources.Count && pending.Count < window)
                    {
                        var source = sources[next++];
                        pending.Enqueue(Task.Run(() =>
                        {
                            slots.Wait(options.CancellationToken);
                            try { return DecodeClip(skeleton, source); }
                            finally { slots.Release(); }
                        }, options.CancellationToken));
                    }
                    yield return pending.Dequeue().GetAwaiter().GetResult();
                }
            }
            finally
            {
                // Only reached early on cancellation or a consumer failure: let the tasks already running finish before the semaphore is disposed.
                foreach (var task in pending)
                    task.ContinueWith(_ => { }, TaskScheduler.Default).GetAwaiter().GetResult();
            }
        }

        /// <summary>Decodes one clip file; format problems are reported on the outcome, never thrown, so one bad clip cannot sink its pack.</summary>
        private static ClipOutcome DecodeClip(HavokSkeleton skeleton, ClipSource source)
        {
            try
            {
                if (!HavokClipBuilder.TryReadFile(File.ReadAllBytes(source.Path), out var file) || !file.HasAnimations)
                    return new ClipOutcome(source, null, null, NonAnimation: true);

                var clips = new List<HavokClip>();
                int dropped = 0;
                foreach (var built in HavokClipBuilder.BuildAll(file, source.Name))
                {
                    if (!string.Equals(built.OriginalSkeletonName, skeleton.Name, StringComparison.Ordinal))
                        throw new HavokSkeletonMismatchException(built.Name, built.OriginalSkeletonName, skeleton.Name);
                    var clip = HavokClipBuilder.DropTracksBeyondBone(built, skeleton.BoneNames.Count, out int clipDropped);
                    HavokGltfWriter.RequireWritable(skeleton, clip);
                    clips.Add(clip);
                    dropped += clipDropped;
                }
                string? warning = dropped == 0 ? null
                    : $"{source.Name}: dropped {dropped} track(s) driving bones beyond the {skeleton.BoneNames.Count}-bone skeleton (clip authored for a larger skeleton)";
                return new ClipOutcome(source, clips, null, NonAnimation: false, warning, dropped);
            }
            catch (HavokException e)
            {
                return new ClipOutcome(source, null, $"{source.Name}: {e.GetType().Name}: {e.Message}", NonAnimation: false);
            }
            catch (Exception e) when (e is IOException or UnauthorizedAccessException)
            {
                return new ClipOutcome(source, null, $"{source.Name}: {e.GetType().Name}: {e.Message}", NonAnimation: false);
            }
        }

        private static void WriteIndex(string path, string skeletonReference, IReadOnlyList<PackResult> written)
        {
            var packs = written.Select(r => r.Index).OfType<PackIndexEntry>()
                .OrderBy(p => p.File.StartsWith(RootPackName + PackSuffix, StringComparison.OrdinalIgnoreCase) ? 0 : 1)
                .ThenBy(p => p.File, StringComparer.OrdinalIgnoreCase)
                .ToList();
            File.WriteAllText(path, JsonSerializer.Serialize(new AnimationIndex(skeletonReference, packs), IndexJsonOptions));
        }

        /// <summary>The skeleton path as seen from the index file, '/' separated.</summary>
        private static string RelativeReference(string indexDir, string skeletonPath) =>
            Path.GetRelativePath(indexDir, skeletonPath).Replace('\\', '/');

        /// <summary>A failed unit must not leave packs, an index or the retired skeleton.skel.glb from an earlier run that no longer match its source.</summary>
        private static void DeleteStaleOutputs(string assetsOut)
        {
            if (!Directory.Exists(assetsOut)) return;
            foreach (string stale in Directory.EnumerateFiles(assetsOut, "*" + PackSuffix))
                File.Delete(stale);
            File.Delete(Path.Combine(assetsOut, IndexOutputName));
            File.Delete(Path.Combine(assetsOut, LegacySkeletonOutputName));
        }
    }
}
