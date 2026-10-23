using System.Text.Encodings.Web;
using System.Text.Json;

namespace AssetConverter.UiLayout
{
    /// <summary>
    /// The head every interface document starts with: <c>format</c> (the schema version the engine checks), <c>kind</c> and
    /// <c>conversion</c> (the <see cref="ConversionStamp"/>: the installer's conversion version, the language and the game files it was
    /// converted from). A document is current only when all of them match; anything else is converted again, never reused.
    /// </summary>
    internal static class UiDocumentHeader
    {
        public const int Format = 1;

        /// <summary>Raised whenever the conversion changes its output for the same input, so existing documents are rewritten.</summary>
        public const int ExtractorVersion = 5;

        /// <summary>Compact output that keeps characters such as <c>&lt;</c> and <c>+</c> of the keys readable instead of escaping them.</summary>
        public static readonly JsonWriterOptions WriterOptions = new() { Indented = false, Encoder = JavaScriptEncoder.UnsafeRelaxedJsonEscaping };

        public const string MovieKind = "movie";
        public const string TranslateKind = "translate";
        public const string FontConfigKind = "fontconfig";

        public static ConversionStamp Stamp(string? language, params SourceFingerprint[] sources) => new(ExtractorVersion, language, sources);

        public static void Write(Utf8JsonWriter json, string kind, ConversionStamp stamp)
        {
            json.WriteNumber("format", Format);
            json.WriteString("kind", kind);
            json.WritePropertyName(ConversionStamp.Property);
            stamp.Write(json);
        }

        /// <returns>Why the document at <paramref name="path"/> is not a current conversion recorded by <paramref name="stamp"/>, or <c>null</c> when it is.</returns>
        public static string? StaleReason(string path, string kind, ConversionStamp stamp)
        {
            if (!File.Exists(path))
                return "it does not exist";
            try
            {
                using var stream = File.OpenRead(path);
                using JsonDocument document = JsonDocument.Parse(stream);
                JsonElement root = document.RootElement;
                if (root.ValueKind != JsonValueKind.Object)
                    return "it is not a JSON object";
                if (!root.TryGetProperty("format", out JsonElement format) || format.ValueKind != JsonValueKind.Number
                    || !format.TryGetInt32(out int number) || number != Format)
                    return "it was written for another document format";
                if (!root.TryGetProperty("kind", out JsonElement written) || written.ValueKind != JsonValueKind.String || written.GetString() != kind)
                    return $"it is not a {kind} document";
                return stamp.DifferenceIn(root);
            }
            catch (JsonException e)
            {
                return $"it is not valid JSON ({e.Message})";
            }
        }
    }
}
