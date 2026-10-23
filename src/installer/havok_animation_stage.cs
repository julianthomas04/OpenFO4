using System.Diagnostics;
using System.Globalization;
using AssetConverter.Havok;
using AssetConverter.Pipeline;
using AssetConverter.Progress;
using SharpBSABA2.BA2Util;

namespace AssetConverter
{
    /// <summary>
    /// Havok animation install stage: extracts the actor .hkx files from Animations.ba2 into staging, then converts
    /// every skeleton unit under staging\Meshes\Actors (archive casing) into glTF skeleton + clip packs under assets\meshes\Actors.
    /// </summary>
    public static class HavokAnimationStage
    {
        public const string ArchiveName = "Fallout4 - Animations.ba2";

        private const string ActorsFolder = "Actors";
        private const string ArchiveMeshesFolder = "Meshes";
        private const string ActorHkxPrefix = "Meshes\\Actors\\";
        private const string HkxExtension = ".hkx";

        public sealed record Summary(
            int Creatures, int CreaturesFailed, int Skeletons, int ClipsOk, int ClipsFailed, int NonAnimationFiles,
            int TracksDropped, int Packs, long Bytes, TimeSpan Elapsed);

        /// <summary>The actor .hkx files of the animation archive, as the staging extraction takes them.</summary>
        internal static StagingSource ActorHkxSource { get; } = new(ArchiveName, $".{ArchiveName}.extracted-hkx", IsActorHkx)
        {
            MissingArchiveMessage = path => $"ERROR: {path} not found; actor .hkx files were not extracted and no animations can be converted.",
        };

        /// <summary>Extracts the actor .hkx files; the marker is only written when every entry extracted cleanly.</summary>
        internal static void ExtractActorHkx(InstallLocations paths, MemoryBudget memory, int progressPhase = -1, CancellationToken ct = default)
        {
            using var extraction = StagingExtraction.Open(paths.GameDataDirectory, paths.StagingRoot, [ActorHkxSource], memory);
            InstallProgress.AddTotal(progressPhase, extraction.WrittenFiles.Count);
            extraction.Run(progressPhase, ct);
        }

        /// <summary>
        /// The staged NIFs whose converted GLBs are the render skeletons the units bind to: the only NIF output <see cref="Convert"/> needs.
        /// Works from the staged .hkx files alone, before any GLB exists.
        /// </summary>
        public static IReadOnlyList<string> RenderSkeletonSources(string stagingRoot, string meshesOut)
        {
            string actorsSource = ActorsSource(stagingRoot);
            string actorsOutput = ActorsOutput(meshesOut);
            return HavokSkeletonUnits.Discover(actorsSource, actorsOutput)
                .Select(unit => HavokSkeletonUnits.RenderSkeletonSource(unit, actorsSource, actorsOutput))
                .Distinct(StringComparer.OrdinalIgnoreCase)
                .ToList();
        }

        private static string ActorsSource(string stagingRoot) => Path.Combine(stagingRoot, ArchiveMeshesFolder, ActorsFolder);

        private static string ActorsOutput(string meshesOut) => Path.Combine(meshesOut, ActorsFolder);

        public static Summary Convert(string stagingRoot, string meshesOut, MemoryBudget memory, CancellationToken ct = default)
        {
            var timer = Stopwatch.StartNew();
            string actorsSource = ActorsSource(stagingRoot);
            var units = HavokSkeletonUnits.Discover(actorsSource, ActorsOutput(meshesOut));
            ReportNothingToConvert(actorsSource, units);
            var orphanClips = CountOrphanClips(actorsSource, units);
            int dop = Math.Min(Environment.ProcessorCount, 12);

            var phase = InstallProgress.BeginPhase(InstallSteps.ConvertAnimations, units.Count);
            var reports = new List<HavokCreatureReport>();
            int failedCreatures = 0;
            foreach (var unit in units)
            {
                ct.ThrowIfCancellationRequested();
                InstallProgress.SetDetail(phase, unit.Label);
                var report = ConvertUnit(unit, dop, memory, ct);
                if (report is null) failedCreatures++; else reports.Add(report);
                InstallProgress.Tick(phase);
            }
            InstallProgress.EndPhase(phase);

            foreach (var (label, count) in orphanClips)
                Console.Error.WriteLine($"  ERROR {label}: no skeleton unit covers this Animations folder; {count} clip file(s) skipped");

            timer.Stop();
            var summary = new Summary(
                Creatures: units.Count, CreaturesFailed: failedCreatures, Skeletons: reports.Count,
                ClipsOk: reports.Sum(r => r.ClipsOk), ClipsFailed: reports.Sum(r => r.ClipsFailed) + orphanClips.Sum(o => o.Count),
                NonAnimationFiles: reports.Sum(r => r.NonAnimationFiles), TracksDropped: reports.Sum(r => r.TracksDropped), Packs: reports.Sum(r => r.Packs),
                Bytes: reports.Sum(r => r.BytesWritten), Elapsed: timer.Elapsed);
            Console.WriteLine(
                $"Havok animations: {summary.Creatures} creatures ({summary.CreaturesFailed} failed), {summary.Skeletons} skeletons, " +
                $"{summary.ClipsOk} clips ok / {summary.ClipsFailed} failed, {summary.NonAnimationFiles} non-animation files skipped, {summary.TracksDropped} out-of-skeleton tracks dropped, " +
                $"{summary.Packs} packs, {ByteFormat.Format(summary.Bytes)}, {summary.Elapsed.TotalSeconds.ToString("F1", CultureInfo.InvariantCulture)}s");
            return summary;
        }

        private static void ReportNothingToConvert(string actorsSource, IReadOnlyCollection<HavokSkeletonUnit> units)
        {
            if (!Directory.Exists(actorsSource))
                Console.Error.WriteLine($"ERROR: staging folder '{actorsSource}' does not exist; no creatures converted (actor .hkx extraction missing or failed).");
            else if (units.Count == 0)
                Console.Error.WriteLine($"ERROR: no {HavokCreatureConverter.AssetsFolder}/{HavokCreatureConverter.SkeletonSourceName} under '{actorsSource}'; no creatures converted.");
        }

        internal static bool IsActorHkx(string archivePath) => ArchivePath.Matches(archivePath, ActorHkxPrefix, HkxExtension);

        private static HavokCreatureReport? ConvertUnit(HavokSkeletonUnit unit, int dop, MemoryBudget memory, CancellationToken ct)
        {
            try
            {
                var report = HavokCreatureConverter.Convert(unit, dop, memory, ct);
                foreach (string warning in report.Warnings)
                    Console.Error.WriteLine($"  WARNING {unit.Label}/{warning}");
                foreach (string error in report.Errors)
                    Console.Error.WriteLine($"  ERROR {unit.Label}/{error}");
                return report;
            }
            catch (HavokException e)
            {
                Console.Error.WriteLine($"  ERROR {unit.Label}: creature skipped: {e.GetType().Name}: {e.Message}");
                return null;
            }
            catch (Exception e) when (e is IOException or UnauthorizedAccessException)
            {
                Console.Error.WriteLine($"  ERROR {unit.Label}: creature skipped: {e.GetType().Name}: {e.Message}");
                return null;
            }
        }

        /// <summary>Animations folders holding clip files that no unit converts (a unit covers its own Animations folder and everything below it).</summary>
        private static List<(string Label, int Count)> CountOrphanClips(string actorsSource, IReadOnlyCollection<HavokSkeletonUnit> units)
        {
            var orphans = new List<(string, int)>();
            if (!Directory.Exists(actorsSource)) return orphans;

            var covered = units.Select(u => Path.TrimEndingDirectorySeparator(Path.GetFullPath(u.AnimationsDir))).ToList();
            foreach (string animations in Directory.EnumerateDirectories(actorsSource, HavokCreatureConverter.AnimationsFolder, SearchOption.AllDirectories)
                         .Order(StringComparer.OrdinalIgnoreCase))
            {
                int count = Directory.EnumerateFiles(Path.GetFullPath(animations), "*" + HkxExtension, SearchOption.AllDirectories)
                    .Count(file => !covered.Any(root => IsUnder(file, root)));
                if (count == 0) continue;
                orphans.Add((Path.GetRelativePath(actorsSource, animations).Replace('\\', '/'), count));
            }
            return orphans;
        }

        private static bool IsUnder(string path, string root) =>
            path.StartsWith(root + Path.DirectorySeparatorChar, StringComparison.OrdinalIgnoreCase);
    }
}
