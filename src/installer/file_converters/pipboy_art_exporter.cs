using System.Globalization;
using System.Text.RegularExpressions;
using AssetConverter.Pipeline;
using AssetConverter.Swf;

namespace AssetConverter
{
    /// <summary>A symbol and the frames of it that are wanted.</summary>
    public sealed record SymbolRequest(int CharacterId, IReadOnlyList<int> Frames);

    /// <summary>A frame file ffdec exported and what it belongs to, for error messages.</summary>
    public sealed record ExportedFrameFile(string Path, string Owner);

    /// <summary>Turns movies and symbols of extracted SWF files into SVG frames through ffdec.</summary>
    public static partial class PipboyArtExporter
    {
        [GeneratedRegex(@"^DefineSprite_(\d+)(_.*)?$")]
        private static partial Regex SpriteFolder();

        /// <summary>
        /// Every timeline frame of a whole movie. A movie whose main timeline is a single frame that hosts a longer nested animation is
        /// exported as that animation, with the frame scripts of the longest sprite.
        /// </summary>
        internal static void ExportMovie(string clipId, string source, string swfPath, SwfMovie movie, string workDirectory, PipboyArtWriter writer,
            MemoryBudget memory, CancellationToken ct, ArtBounds? contentBounds = null)
        {
            int longestSprite = movie.Characters.Values.OfType<SwfSprite>().Select(sprite => sprite.Timeline.DeclaredFrameCount).DefaultIfEmpty(0).Max();
            bool nestedAnimation = movie.Root.DeclaredFrameCount == 1 && longestSprite > 1;
            int frameCount = nestedAnimation ? longestSprite : movie.Root.DeclaredFrameCount;

            Directory.CreateDirectory(workDirectory);
            var arguments = new List<string> { "-format", "frame:svg", "-ignorebackground" };
            if (nestedAnimation) arguments.AddRange(["-sublength", frameCount.ToString(CultureInfo.InvariantCulture)]);
            arguments.AddRange(["-export", "frame", workDirectory, swfPath]);
            Ffdec.RunExporting(memory, workDirectory, [.. arguments], ct);

            string frameDirectory = nestedAnimation ? Path.Combine(workDirectory, "1") : workDirectory;
            var frames = NumberedFrameFiles(frameDirectory, clipId);
            if (frames.Count != frameCount)
                throw new PipboyArtException($"Clip '{clipId}': ffdec exported {frames.Count} frame(s) of '{source}', expected {frameCount}.");

            var scripts = new PipboyArtScripts(movie, source);
            var frameNumbers = Enumerable.Range(1, frameCount).ToList();
            var script = nestedAnimation ? NestedAnimationScript(movie, scripts, longestSprite, frameNumbers, clipId) : scripts.Of(null, frameNumbers);
            var stage = movie.Stage;
            var bounds = contentBounds ?? new ArtBounds(Twips.ToPixels(stage.Xmin), Twips.ToPixels(stage.Ymin), Twips.ToPixels(stage.Xmax), Twips.ToPixels(stage.Ymax));
            double framesPerSecond = FrameRate(movie, source);
            AddMovieClip(writer, memory, frames, bounds,
                rebased => new PipboyArtClip(clipId, source, null, ArtColour.Mask, framesPerSecond, script, bounds, rebased), ct);
        }

        /// <summary>
        /// Reads the frames of a movie clip one at a time, keeping only their copies rebased to <paramref name="bounds"/>, and adds the
        /// clip to <paramref name="writer"/>, all under a lease sized from the frame files.
        /// </summary>
        internal static void AddMovieClip(PipboyArtWriter writer, MemoryBudget memory, IReadOnlyList<ExportedFrameFile> frames, ArtBounds bounds,
            Func<IReadOnlyList<string>, PipboyArtClip> clip, CancellationToken ct)
        {
            using var lease = memory.Reserve(MemoryEstimates.PipboyMovieClipBytes(TotalLength(frames)), ct);
            var rebased = new List<string>(frames.Count);
            foreach (var frame in frames)
                rebased.Add(ParseFile(frame).Rebase(bounds));
            writer.Add(clip(rebased));
        }

        /// <summary>
        /// Reads the frames of a symbol clip, each file once, builds the clip from them and adds it to <paramref name="writer"/>, all
        /// under a lease sized from the frame files.
        /// </summary>
        internal static void AddSymbolClip(PipboyArtWriter writer, MemoryBudget memory, IReadOnlyList<ExportedFrameFile> frames,
            Func<IReadOnlyList<SvgFrame>, PipboyArtClip> clip, CancellationToken ct)
        {
            using var lease = memory.Reserve(MemoryEstimates.PipboySymbolClipBytes(TotalLength(frames)), ct);
            var parsed = new Dictionary<string, SvgFrame>(StringComparer.OrdinalIgnoreCase);
            var read = new List<SvgFrame>(frames.Count);
            foreach (var frame in frames)
            {
                if (!parsed.TryGetValue(frame.Path, out var svg))
                    parsed.Add(frame.Path, svg = ParseFile(frame));
                read.Add(svg);
            }
            writer.Add(clip(read));
        }

        static long TotalLength(IEnumerable<ExportedFrameFile> frames) => frames.Sum(frame => new FileInfo(frame.Path).Length);

        /// <summary>The frame scripts of the sprites that run the nested animation; several of them must agree.</summary>
        static IReadOnlyList<ClipScriptStep> NestedAnimationScript(SwfMovie movie, PipboyArtScripts scripts, int frameCount, IReadOnlyList<int> frames, string clipId)
        {
            var distinct = movie.Characters.Values.OfType<SwfSprite>().Where(sprite => sprite.Timeline.DeclaredFrameCount == frameCount)
                .Select(sprite => scripts.Of(sprite.Id, frames)).DistinctBy(steps => string.Join(';', steps)).ToList();
            return distinct.Count == 1
                ? distinct[0]
                : throw new PipboyArtException($"Clip '{clipId}': the sprites of its nested animation run {distinct.Count} different frame scripts.");
        }

        /// <summary>The playback rate the movie's header states.</summary>
        internal static double FrameRate(SwfMovie movie, string source) =>
            movie.FrameRate > 0 && double.IsFinite(movie.FrameRate)
                ? movie.FrameRate
                : throw new PipboyArtException($"'{source}' states the frame rate {movie.FrameRate}, which is not a positive number.");

        /// <summary>The files of the requested frames of symbols of one movie, keyed by character id and frame number.</summary>
        public static IReadOnlyDictionary<(int CharacterId, int Frame), ExportedFrameFile> ExportSymbols(
            string swfPath, IReadOnlyCollection<SymbolRequest> requests, string workDirectory, MemoryBudget memory, CancellationToken ct)
        {
            Directory.CreateDirectory(workDirectory);
            string ids = string.Join(',', requests.Select(r => r.CharacterId.ToString(CultureInfo.InvariantCulture)));
            string frames = string.Join(',', requests.Select(r => $"{r.CharacterId}:{FrameRanges(r.Frames)}"));
            Ffdec.RunExporting(memory, workDirectory,
                ["-format", "sprite:svg,shape:svg", "-ignorebackground", "-selectid", ids, "-select", frames, "-export", "sprite,shape", workDirectory, swfPath], ct);

            var exported = new Dictionary<(int, int), ExportedFrameFile>();
            string spriteRoot = Path.Combine(workDirectory, "sprites");
            if (Directory.Exists(spriteRoot))
            {
                foreach (string folder in Directory.GetDirectories(spriteRoot))
                {
                    var match = SpriteFolder().Match(Path.GetFileName(folder));
                    if (!match.Success) continue;
                    int characterId = int.Parse(match.Groups[1].Value, CultureInfo.InvariantCulture);
                    foreach (string file in Directory.GetFiles(folder, "*.svg"))
                        exported[(characterId, int.Parse(Path.GetFileNameWithoutExtension(file), CultureInfo.InvariantCulture))] =
                            new ExportedFrameFile(file, $"symbol {characterId} of '{Path.GetFileName(swfPath)}'");
                }
            }
            string shapeRoot = Path.Combine(workDirectory, "shapes");
            if (Directory.Exists(shapeRoot))
            {
                foreach (string file in Directory.GetFiles(shapeRoot, "*.svg"))
                    exported[(int.Parse(Path.GetFileNameWithoutExtension(file), CultureInfo.InvariantCulture), 1)] =
                        new ExportedFrameFile(file, $"shape of '{Path.GetFileName(swfPath)}'");
            }
            return exported;
        }

        /// <summary>The frame numbers (ascending, distinct) as the comma separated runs ffdec's <c>-select</c> takes: <c>1-4,7</c>.</summary>
        public static string FrameRanges(IEnumerable<int> frames)
        {
            var runs = new List<string>();
            int[] sorted = [.. frames.Distinct().Order()];
            for (int start = 0; start < sorted.Length;)
            {
                int end = start;
                while (end + 1 < sorted.Length && sorted[end + 1] == sorted[end] + 1) end++;
                runs.Add(end == start
                    ? sorted[start].ToString(CultureInfo.InvariantCulture)
                    : string.Create(CultureInfo.InvariantCulture, $"{sorted[start]}-{sorted[end]}"));
                start = end + 1;
            }
            return string.Join(',', runs);
        }

        /// <summary>
        /// Exported frames of one symbol as a clip. The bounds cover every frame that shows something; a frame without content stays an
        /// empty image of those bounds, so frame numbers keep their meaning.
        /// </summary>
        public static PipboyArtClip SymbolClip(string clipId, string source, string? illustrates, ArtColour colour, double framesPerSecond,
            IReadOnlyList<ClipScriptStep> script, IReadOnlyList<SvgFrame> frames, IReadOnlyList<double>? offset = null, IReadOnlyList<double>? tint = null)
        {
            ArtBounds? bounds = null;
            foreach (var frame in frames)
                if (frame.Bounds is { } frameBounds)
                    bounds = bounds is { } existing ? existing.Union(frameBounds) : frameBounds;
            if (bounds is not { } covered)
                throw new PipboyArtException($"Clip '{clipId}': the symbol of '{source}' exported no content.");
            return new PipboyArtClip(clipId, source, illustrates, colour, framesPerSecond, script, covered,
                frames.Select(f => f.Rebase(covered)).ToList(), offset, tint);
        }

        static List<ExportedFrameFile> NumberedFrameFiles(string directory, string clipId)
        {
            if (!Directory.Exists(directory))
                throw new PipboyArtException($"Clip '{clipId}': ffdec produced no frames.");
            return Directory.GetFiles(directory, "*.svg")
                .OrderBy(file => int.Parse(Path.GetFileNameWithoutExtension(file), CultureInfo.InvariantCulture))
                .Select(file => new ExportedFrameFile(file, $"clip '{clipId}'"))
                .ToList();
        }

        static SvgFrame ParseFile(ExportedFrameFile frame) => ParseFile(frame.Path, frame.Owner);

        public static SvgFrame ParseFile(string file, string owner)
        {
            try
            {
                return SvgFrame.Parse(File.ReadAllText(file));
            }
            catch (SvgFormatException failure)
            {
                throw new PipboyArtException($"Unusable ffdec frame '{file}' of {owner}: {failure.Message}");
            }
        }
    }
}
