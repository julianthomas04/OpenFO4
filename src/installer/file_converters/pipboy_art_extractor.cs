using System.Diagnostics;
using System.Globalization;
using AssetConverter.Pipeline;
using AssetConverter.Swf;

namespace AssetConverter
{
    /// <summary>
    /// Exports the Pip-Boy's vector art from <c>Fallout4 - Interface.ba2</c> as SVG frames plus a manifest (see
    /// docs/installer/interface.md): the pictograms, paper doll, map markers, perk-chart cards, limb condition figures
    /// and every Vault Boy movie. Skipped when the art folder holds a complete manifest of the current format and its sidecar records the
    /// current <see cref="ConversionStamp"/> (this extractor's version, the ffdec version and every movie read). Every failure throws,
    /// naming the movie it happened in; the output folder is published by one directory swap, so it only ever holds a complete export.
    /// </summary>
    public static class PipboyArtExtractor
    {
        const string InterfaceRoot = @"Interface\";
        const string ComponentsRoot = @"Components\";

        /// <summary>Raised whenever the export changes its output for the same game files.</summary>
        internal const int ExtractorVersion = 4;

        /// <summary>Every export job runs its own ffdec JVM; more than two at once exhausts memory on a loaded machine.</summary>
        const int MaxConcurrentFfdecProcesses = 2;

        /// <returns><c>true</c> when the art was exported, <c>false</c> when it was already present.</returns>
        public static bool Extract(string fo4Location, string stagingRoot, string artRoot, MemoryBudget memory, CancellationToken ct = default)
        {
            var movies = ReadMovies(fo4Location);
            var plan = PipboyArtPlan.Build(movies);
            var stamp = new ConversionStamp(ExtractorVersion, null,
                [.. movies.Values.OrderBy(m => m.Source, StringComparer.OrdinalIgnoreCase).Select(m => SourceFingerprint.Of(InterfaceRoot + m.Source, m.Content))],
                Ffdec.Version(memory, ct));
            string? reason = stamp.SidecarStaleReason(artRoot) ?? PipboyArtWriter.FormatProblem(Path.Combine(artRoot, InstallLayout.PipboyArtManifestFileName));
            if (reason is null)
            {
                Console.WriteLine($"Pip-Boy art already present at {artRoot}; skipping.");
                return false;
            }
            Console.WriteLine($"Pip-Boy art at {artRoot} is exported again: {reason}.");

            var timer = Stopwatch.StartNew();
            string workRoot = Path.Combine(stagingRoot, "pipboy_art_work");
            try
            {
                Scratch.TryDeleteDir(workRoot);
                foreach (ArtMovie movie in movies.Values)
                {
                    string local = LocalPath(stagingRoot, movie);
                    Directory.CreateDirectory(Path.GetDirectoryName(local)!);
                    File.WriteAllBytes(local, movie.Content);
                }

                int clipCount = 0;
                AtomicPublish.Folder(artRoot, folder =>
                {
                    var writer = new PipboyArtWriter(folder);
                    try
                    {
                        ConversionGuard.RunAll(
                            SymbolJobs(movies, plan, stagingRoot, workRoot, writer, memory, ct).Concat(MovieJobs(movies, plan, stagingRoot, workRoot, writer, memory, ct)),
                            MaxConcurrentFfdecProcesses,
                            failures => new PipboyArtException(ConversionGuard.Describe(failures), failures[0]));
                    }
                    catch (PipboyArtException failure) when (ct.IsCancellationRequested)
                    {
                        throw new OperationCanceledException(failure.Message, failure, ct);
                    }
                    clipCount = writer.Complete(plan.ClipIds);
                    stamp.WriteSidecar(folder);
                });
                long bytes = Directory.EnumerateFiles(artRoot, "*", SearchOption.AllDirectories).Sum(f => new FileInfo(f).Length);
                int files = Directory.EnumerateFiles(artRoot, "*", SearchOption.AllDirectories).Count();
                Console.WriteLine($"Exported {clipCount} Pip-Boy art clip(s): {files} file(s), {bytes / (1024.0 * 1024.0):F1} MB in {timer.Elapsed:mm\\:ss}.");
                return true;
            }
            finally
            {
                foreach (ArtMovie movie in movies.Values) Scratch.TryDeleteFile(LocalPath(stagingRoot, movie));
                Scratch.TryDeleteDir(Path.Combine(stagingRoot, "Interface", "Components"));
                Scratch.TryDeleteDir(workRoot);
            }
        }

        static string LocalPath(string stagingRoot, ArtMovie movie) => Path.Combine(stagingRoot, InterfaceRoot, movie.Source);

        /// <summary>Reads the movies the export uses from the interface archive, keyed by their path below <c>Interface\</c>.</summary>
        internal static Dictionary<string, ArtMovie> ReadMovies(string fo4Location)
        {
            string[] pages = [PipboyArtCatalog.InventoryPage, PipboyArtCatalog.StatsPage, PipboyArtCatalog.DataPage,
                PipboyArtCatalog.MapPage, PipboyArtCatalog.Shell, PipboyArtCatalog.LevelUpMenu, PipboyArtCatalog.LoadingScreen];
            bool Wanted(string archivePath)
            {
                if (!archivePath.StartsWith(InterfaceRoot, StringComparison.OrdinalIgnoreCase)) return false;
                string relative = archivePath[InterfaceRoot.Length..];
                if (!relative.EndsWith(".swf", StringComparison.OrdinalIgnoreCase)) return false;
                return pages.Contains(relative, StringComparer.OrdinalIgnoreCase)
                    || relative.StartsWith(ComponentsRoot, StringComparison.OrdinalIgnoreCase);
            }

            var movies = new Dictionary<string, ArtMovie>(StringComparer.OrdinalIgnoreCase);
            foreach ((string path, byte[] content) in InterfaceArchive.ReadMatching(fo4Location, Wanted))
            {
                string source = path[InterfaceRoot.Length..];
                if (!movies.TryAdd(source, new ArtMovie(source, content)))
                    throw new PipboyArtException($"The interface archive holds '{path}' twice.");
            }
            return movies;
        }

        static T Guard<T>(ArtMovie movie, Func<T> work) =>
            ConversionGuard.Run(movie.Source, work, (message, cause) => new PipboyArtException(message, cause));

        static IEnumerable<Action> SymbolJobs(IReadOnlyDictionary<string, ArtMovie> movies, PipboyArtPlan plan, string stagingRoot, string workRoot, PipboyArtWriter writer,
            MemoryBudget memory, CancellationToken ct)
        {
            int job = 0;
            foreach (var group in plan.Symbols.GroupBy(work => work.Movie, StringComparer.OrdinalIgnoreCase))
            {
                string work = Path.Combine(workRoot, string.Create(CultureInfo.InvariantCulture, $"symbols_{job++}"));
                ArtMovie movie = PipboyArtPlan.Require(movies, group.Key);
                var list = group.ToList();
                yield return () => Guard(movie, () => ExportSymbolClips(movie, LocalPath(stagingRoot, movie), list, work, writer, memory, ct));
            }
        }

        static bool ExportSymbolClips(ArtMovie movie, string localPath, List<SymbolWork> works, string work, PipboyArtWriter writer, MemoryBudget memory, CancellationToken ct)
        {
            var tree = new PipboySwfTree(SwfMovieReader.Read(movie.Content));
            var scripts = new PipboyArtScripts(tree.Movie, movie.Source);
            double framesPerSecond = PipboyArtExporter.FrameRate(tree.Movie, movie.Source);
            var resolved = PipboyArtPlan.Resolve(movie, tree, works);
            var requests = resolved.GroupBy(r => r.Symbol.CharacterId)
                .Select(group => new SymbolRequest(group.Key, group.SelectMany(r => r.Symbol.Frames).Distinct().Order().ToList()))
                .ToList();
            var exported = PipboyArtExporter.ExportSymbols(localPath, requests, Path.Combine(work, "export"), memory, ct);
            foreach (var (item, symbol) in resolved)
            {
                var files = symbol.Frames.Select(frame => exported.TryGetValue((symbol.CharacterId, frame), out var file)
                    ? file
                    : throw new PipboyArtException($"Clip '{item.ClipId}': ffdec exported no frame {frame} of a symbol of '{movie.Source}'.")).ToList();
                var script = tree.IsSprite(symbol.CharacterId) ? scripts.Of(symbol.CharacterId, symbol.Frames) : [];
                PipboyArtExporter.AddSymbolClip(writer, memory, files, frames => PipboyArtExporter.SymbolClip(item.ClipId, movie.Source, item.Illustrates,
                    item.Colour, framesPerSecond, script, frames, symbol.Offset, symbol.Tint), ct);
            }
            return true;
        }

        static IEnumerable<Action> MovieJobs(IReadOnlyDictionary<string, ArtMovie> movies, PipboyArtPlan plan, string stagingRoot, string workRoot, PipboyArtWriter writer,
            MemoryBudget memory, CancellationToken ct)
        {
            int job = 0;
            foreach (var source in plan.Movies)
            {
                string work = Path.Combine(workRoot, string.Create(CultureInfo.InvariantCulture, $"movie_{job++}"));
                ArtMovie movie = PipboyArtPlan.Require(movies, source.Movie);
                yield return () => Guard(movie, () =>
                {
                    SwfMovie parsed = SwfMovieReader.Read(movie.Content);
                    PipboyArtExporter.ExportMovie(source.ClipId, movie.Source, LocalPath(stagingRoot, movie), parsed, work, writer, memory, ct,
                        source.TightBounds ? ContentBounds(parsed) : null);
                    return true;
                });
            }
        }

        static ArtBounds ContentBounds(SwfMovie movie)
        {
            var rect = new PipboySwfTree(movie).ContentBounds();
            return new ArtBounds(Twips.ToPixels(rect.Xmin), Twips.ToPixels(rect.Ymin), Twips.ToPixels(rect.Xmax), Twips.ToPixels(rect.Ymax));
        }
    }
}
