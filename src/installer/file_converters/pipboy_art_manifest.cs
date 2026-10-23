using System.IO.Compression;
using System.Security.Cryptography;
using System.Text;
using System.Text.Json;
using System.Text.Json.Serialization;

namespace AssetConverter
{
    public sealed class PipboyArtException(string message, Exception? inner = null) : ConversionStepException(message, inner) { }

    public enum ArtColour
    {
        Mask,
        Full,
    }

    /// <summary>
    /// A clip ready to publish: frames are complete SVG documents sharing the viewBox <see cref="Bounds"/>. <paramref name="Offset"/> is where a
    /// part's registration point sits inside the composite it belongs to (pixels); <paramref name="Tint"/> is the red, green, blue and alpha
    /// multiplier (0..1) the composite applies to it. <paramref name="Script"/> is what the clip's own frame scripts do to its playback;
    /// without steps it plays on and wraps round.
    /// </summary>
    public sealed record PipboyArtClip(
        string Id,
        string Source,
        string? Illustrates,
        ArtColour Colour,
        double FramesPerSecond,
        IReadOnlyList<ClipScriptStep> Script,
        ArtBounds Bounds,
        IReadOnlyList<string> Frames,
        IReadOnlyList<double>? Offset = null,
        IReadOnlyList<double>? Tint = null);

    /// <summary>
    /// Writes the frame files and <c>art_manifest.json</c> in the format the engine's art library validates. Each clip's frames go to disk
    /// as it is added, so only the small manifest entries stay in memory until <see cref="Complete"/>; <see cref="Add"/> may be called
    /// from several threads.
    /// </summary>
    public sealed class PipboyArtWriter
    {
        public const int ManifestFormat = 3;
        const int FrameNumberDigits = 4;
        const double MinimumGzipRatio = 3.0;
        const string RawExtension = ".svg";
        const string CompressedExtension = ".svgz";

        static readonly JsonSerializerOptions ManifestJson = new()
        {
            WriteIndented = false,
            DefaultIgnoreCondition = JsonIgnoreCondition.WhenWritingNull,
        };

        sealed record ManifestFile(
            [property: JsonPropertyName("format")] int Format,
            [property: JsonPropertyName("clips")] IReadOnlyList<ManifestClip> Clips);

        sealed record ManifestClip(
            [property: JsonPropertyName("id")] string Id,
            [property: JsonPropertyName("source")] string Source,
            [property: JsonPropertyName("illustrates")] string? Illustrates,
            [property: JsonPropertyName("colour")] string Colour,
            [property: JsonPropertyName("fps")] double Fps,
            [property: JsonPropertyName("script")] IReadOnlyList<ManifestScriptStep> Script,
            [property: JsonPropertyName("bounds")] double[] Bounds,
            [property: JsonPropertyName("frames")] IReadOnlyList<string> Frames,
            [property: JsonPropertyName("offset")] IReadOnlyList<double>? Offset,
            [property: JsonPropertyName("tint")] IReadOnlyList<double>? Tint);

        sealed record ManifestScriptStep(
            [property: JsonPropertyName("frame")] int Frame,
            [property: JsonPropertyName("action")] string Action,
            [property: JsonPropertyName("target")] int? Target);

        /// <summary>The folder below the art root that holds a clip's frames: the id with path-hostile characters replaced.</summary>
        public static string ClipFolder(string clipId)
        {
            char[] invalid = Path.GetInvalidFileNameChars();
            var segments = clipId.Split('/').Select(segment =>
            {
                var cleaned = new string(segment.Select(c => invalid.Contains(c) ? '_' : c).ToArray()).TrimEnd(' ', '.');
                return cleaned.Length == 0 ? "_" : cleaned;
            });
            return string.Join('/', segments);
        }

        /// <returns>Why the manifest at <paramref name="manifestPath"/> is not one of the format this installer writes, or <c>null</c> when it is.</returns>
        public static string? FormatProblem(string manifestPath)
        {
            if (!File.Exists(manifestPath))
                return "it has no manifest";
            try
            {
                using var manifest = JsonDocument.Parse(File.ReadAllText(manifestPath));
                JsonElement root = manifest.RootElement;
                if (!(root.ValueKind == JsonValueKind.Object && root.TryGetProperty("format", out var format)
                    && format.ValueKind == JsonValueKind.Number && format.TryGetInt32(out int version) && version == ManifestFormat))
                    return $"its manifest is not of format {ManifestFormat}";
                return MissingFrame(Path.GetDirectoryName(manifestPath)!, root) is { } missing
                    ? $"its frame file '{missing}' is missing or empty"
                    : null;
            }
            catch (JsonException e)
            {
                return $"its manifest is not valid JSON ({e.Message})";
            }
        }

        static string? MissingFrame(string artRoot, JsonElement manifest)
        {
            if (!manifest.TryGetProperty("clips", out var clips) || clips.ValueKind != JsonValueKind.Array)
                return "clips";
            foreach (JsonElement clip in clips.EnumerateArray())
            {
                if (clip.ValueKind != JsonValueKind.Object || !clip.TryGetProperty("frames", out var frames) || frames.ValueKind != JsonValueKind.Array)
                    return "frames";
                foreach (JsonElement frame in frames.EnumerateArray())
                {
                    string? relative = frame.ValueKind == JsonValueKind.String ? frame.GetString() : null;
                    if (string.IsNullOrEmpty(relative))
                        return "frames";
                    var info = new FileInfo(Path.Combine(artRoot, relative));
                    if (!info.Exists || info.Length == 0)
                        return relative;
                }
            }
            return null;
        }

        readonly string _artRoot;
        readonly object _gate = new();
        readonly HashSet<string> _ids = new(StringComparer.Ordinal);
        readonly HashSet<string> _folders = new(StringComparer.OrdinalIgnoreCase);
        readonly List<ManifestClip> _clips = [];

        public PipboyArtWriter(string artRoot)
        {
            _artRoot = artRoot;
            Directory.CreateDirectory(artRoot);
        }

        /// <summary>Validates the clip and writes its frame files.</summary>
        public void Add(PipboyArtClip clip)
        {
            ValidateClip(clip);
            lock (_gate)
            {
                if (!_ids.Add(clip.Id))
                    throw new PipboyArtException($"Clip id '{clip.Id}' appears twice.");
                if (!_folders.Add(ClipFolder(clip.Id)))
                    throw new PipboyArtException($"Clip id '{clip.Id}' shares its frame folder with another clip.");
            }

            ManifestClip written = WriteClip(clip);
            lock (_gate)
                _clips.Add(written);
        }

        /// <summary>Checks that every required clip was added and writes the manifest, its clips ordered by id.</summary>
        /// <returns>The number of clips in the manifest.</returns>
        public int Complete(IReadOnlyCollection<string> requiredClipIds)
        {
            ManifestClip[] ordered;
            lock (_gate)
            {
                var missing = requiredClipIds.Where(id => !_ids.Contains(id)).ToList();
                if (missing.Count > 0)
                    throw new PipboyArtException($"Required clip(s) missing: {string.Join(", ", missing)}.");
                ordered = [.. _clips.OrderBy(clip => clip.Id, StringComparer.Ordinal)];
            }

            string manifestPath = Path.Combine(_artRoot, InstallLayout.PipboyArtManifestFileName);
            File.WriteAllText(manifestPath, JsonSerializer.Serialize(new ManifestFile(ManifestFormat, ordered), ManifestJson));
            return ordered.Length;
        }

        static void ValidateClip(PipboyArtClip clip)
        {
            if (clip.Frames.Count == 0)
                throw new PipboyArtException($"Clip '{clip.Id}' has no frames.");
            if (!(clip.FramesPerSecond > 0))
                throw new PipboyArtException($"Clip '{clip.Id}' has a non-positive frame rate.");
            if (!(clip.Bounds.Width > 0) || !(clip.Bounds.Height > 0))
                throw new PipboyArtException($"Clip '{clip.Id}' has degenerate bounds.");
            if (clip.Script.Any(step => step.Frame < 1 || step.Frame > clip.Frames.Count || step.Target < 1 || step.Target > clip.Frames.Count))
                throw new PipboyArtException($"Clip '{clip.Id}' has a script step outside its {clip.Frames.Count} frame(s).");
        }

        ManifestClip WriteClip(PipboyArtClip clip)
        {
            string folder = ClipFolder(clip.Id);
            Directory.CreateDirectory(Path.Combine(_artRoot, folder));

            var written = new Dictionary<string, string>(StringComparer.Ordinal);
            var framePaths = new List<string>(clip.Frames.Count);
            for (int index = 0; index < clip.Frames.Count; index++)
            {
                byte[] svg = Encoding.UTF8.GetBytes(clip.Frames[index]);
                string hash = Convert.ToHexString(SHA256.HashData(svg));
                if (!written.TryGetValue(hash, out string? relativePath))
                {
                    relativePath = WriteFrame(folder, index, svg);
                    written[hash] = relativePath;
                }
                framePaths.Add(relativePath);
            }

            return new ManifestClip(clip.Id, clip.Source, clip.Illustrates, clip.Colour.ToString().ToLowerInvariant(),
                clip.FramesPerSecond, [.. clip.Script.Select(step => new ManifestScriptStep(step.Frame, step.Action, step.Target))],
                clip.Bounds.ToArray(), framePaths, clip.Offset, clip.Tint);
        }

        string WriteFrame(string folder, int index, byte[] raw)
        {
            byte[] compressed = Gzip(raw);
            bool useCompressed = compressed.Length * MinimumGzipRatio <= raw.Length;
            string name = index.ToString("D" + FrameNumberDigits, System.Globalization.CultureInfo.InvariantCulture)
                + (useCompressed ? CompressedExtension : RawExtension);
            string relativePath = $"{folder}/{name}";
            File.WriteAllBytes(Path.Combine(_artRoot, relativePath), useCompressed ? compressed : raw);
            return relativePath;
        }

        static byte[] Gzip(byte[] data)
        {
            using var output = new MemoryStream();
            using (var gzip = new GZipStream(output, CompressionLevel.SmallestSize, leaveOpen: true))
                gzip.Write(data);
            return output.ToArray();
        }
    }
}
