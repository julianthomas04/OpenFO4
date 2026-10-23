using System.Security.Cryptography;
using System.Text.Encodings.Web;
using System.Text.Json;

namespace AssetConverter
{
    /// <summary>A game file a conversion read: its archive entry, byte size and SHA-256.</summary>
    internal sealed record SourceFingerprint(string Entry, long Size, string Sha256)
    {
        public static SourceFingerprint Of(string entry, byte[] content) =>
            new(entry, content.LongLength, Convert.ToHexString(SHA256.HashData(content)).ToLowerInvariant());
    }

    /// <summary>
    /// What an output was converted from and by: the step's extractor version, the installed language when the output depends on it, the
    /// version of ffdec when the output went through it, and every game file the step read. An output is current only when the stamp it records equals the one the step would record now;
    /// anything else is converted again, never reused.
    /// </summary>
    internal sealed record ConversionStamp(int Extractor, string? Language, IReadOnlyList<SourceFingerprint> Sources, string? FfdecVersion = null, string? FfmpegVersion = null)
    {
        /// <summary>The property a document records its stamp under.</summary>
        public const string Property = "conversion";

        /// <summary>The file beside an output that records the stamp of an output which cannot hold it itself.</summary>
        public const string SidecarSuffix = ".stamp";

        private static readonly JsonWriterOptions Compact = new() { Encoder = JavaScriptEncoder.UnsafeRelaxedJsonEscaping };

        public void Write(Utf8JsonWriter json)
        {
            json.WriteStartObject();
            json.WriteNumber("extractor", Extractor);
            if (Language is not null)
                json.WriteString("language", Language);
            if (FfdecVersion is not null)
                json.WriteString("ffdec", FfdecVersion);
            if (FfmpegVersion is not null)
                json.WriteString("ffmpeg", FfmpegVersion);
            json.WritePropertyName("sources");
            WriteSources(json);
            json.WriteEndObject();
        }

        private void WriteSources(Utf8JsonWriter json)
        {
            json.WriteStartArray();
            foreach (SourceFingerprint source in Sources)
            {
                json.WriteStartObject();
                json.WriteString("entry", source.Entry);
                json.WriteNumber("size", source.Size);
                json.WriteString("sha256", source.Sha256);
                json.WriteEndObject();
            }
            json.WriteEndArray();
        }

        private string SourcesJson() => Serialize(WriteSources);

        public string ToJson() => Serialize(Write);

        private static string Serialize(Action<Utf8JsonWriter> write)
        {
            using var buffer = new MemoryStream();
            using (var json = new Utf8JsonWriter(buffer, Compact))
                write(json);
            return System.Text.Encoding.UTF8.GetString(buffer.ToArray());
        }

        /// <returns>Why <paramref name="recorded"/> is not this stamp, or <c>null</c> when it is.</returns>
        public string? Difference(JsonElement recorded)
        {
            if (recorded.ValueKind != JsonValueKind.Object)
                return "its conversion record is not a JSON object";
            if (!recorded.TryGetProperty("extractor", out JsonElement extractor) || extractor.ValueKind != JsonValueKind.Number
                || !extractor.TryGetInt32(out int version) || version != Extractor)
                return "it was converted by another extractor version";
            bool recordsLanguage = recorded.TryGetProperty("language", out JsonElement language);
            if (recordsLanguage != (Language is not null)
                || (recordsLanguage && (language.ValueKind != JsonValueKind.String || language.GetString() != Language)))
                return "it was converted for another language";
            if (!RecordsTool(recorded, "ffdec", FfdecVersion))
                return "it was converted by another ffdec version";
            if (!RecordsTool(recorded, "ffmpeg", FfmpegVersion))
                return "it was converted by another ffmpeg version";
            if (!recorded.TryGetProperty("sources", out JsonElement sources) || sources.GetRawText() != SourcesJson())
                return "the game files it was converted from have changed";
            return null;
        }

        private static bool RecordsTool(JsonElement recorded, string property, string? expectedVersion)
        {
            bool records = recorded.TryGetProperty(property, out JsonElement version);
            return records == (expectedVersion is not null)
                && (!records || (version.ValueKind == JsonValueKind.String && version.GetString() == expectedVersion));
        }

        /// <returns>Why the document root <paramref name="root"/> does not record this stamp, or <c>null</c> when it does.</returns>
        public string? DifferenceIn(JsonElement root) =>
            root.TryGetProperty(Property, out JsonElement recorded) ? Difference(recorded) : "it records no conversion";

        /// <returns>Why the output at <paramref name="outputPath"/> with its sidecar stamp is not current, or <c>null</c> when it is.</returns>
        public string? SidecarStaleReason(string outputPath)
        {
            if (!File.Exists(outputPath) && !Directory.Exists(outputPath))
                return "it does not exist";
            return StaleReasonAt(SidecarOf(outputPath));
        }

        /// <returns>Why the stamp file <paramref name="stampFile"/> does not record this stamp, or <c>null</c> when it does.</returns>
        public string? StaleReasonAt(string stampFile)
        {
            if (!File.Exists(stampFile))
                return "it records no conversion";
            try
            {
                using JsonDocument document = JsonDocument.Parse(File.ReadAllText(stampFile));
                return Difference(document.RootElement);
            }
            catch (JsonException e)
            {
                return $"its conversion record is not valid JSON ({e.Message})";
            }
        }

        /// <summary>Records this stamp beside <paramref name="outputPath"/>; a crash never leaves half a stamp.</summary>
        public void WriteSidecar(string outputPath) => WriteTo(SidecarOf(outputPath));

        /// <summary>Records this stamp in <paramref name="stampFile"/>; a crash never leaves half a stamp.</summary>
        public void WriteTo(string stampFile) => AtomicPublish.File(stampFile, ToJson());

        /// <summary>The sidecar of a file output sits beside it; the sidecar of a folder output sits inside it, so the folder is published with it.</summary>
        public static string SidecarOf(string outputPath) =>
            Directory.Exists(outputPath) ? Path.Combine(outputPath, SidecarSuffix) : outputPath + SidecarSuffix;
    }
}
