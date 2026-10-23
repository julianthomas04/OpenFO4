using System.Text.Json;

namespace AssetConverter.UiLayout
{
    /// <summary>
    /// The font configuration of the installed language as the engine reads it: font libraries, class-to-family maps, the vertical metrics
    /// of every mapped font, character sets.
    /// </summary>
    internal static class FontConfigDocument
    {
        public static byte[] Write(FontConfig config, IReadOnlyList<FontMetrics> fonts, string language, ConversionStamp stamp)
        {
            using var buffer = new MemoryStream();
            using (var json = new Utf8JsonWriter(buffer, UiDocumentHeader.WriterOptions))
            {
                json.WriteStartObject();
                UiDocumentHeader.Write(json, UiDocumentHeader.FontConfigKind, stamp);
                json.WriteString("language", language);
                json.WriteStartArray("fontLibraries");
                foreach (string library in config.FontLibraries)
                    json.WriteStringValue(library);
                json.WriteEndArray();
                json.WriteStartArray("maps");
                foreach (FontConfigMap map in config.Maps)
                {
                    json.WriteStartObject();
                    json.WriteString("class", map.FontClass);
                    json.WriteString("family", map.Family);
                    json.WriteString("style", map.Style);
                    json.WriteEndObject();
                }
                json.WriteEndArray();
                json.WriteStartArray("fonts");
                foreach (FontMetrics font in fonts)
                {
                    json.WriteStartObject();
                    json.WriteString("family", font.Family);
                    json.WriteString("style", font.Style);
                    json.WriteString("library", font.Library);
                    json.WriteNumber("emSize", font.EmSize);
                    json.WriteNumber("ascent", font.Ascent);
                    json.WriteNumber("descent", font.Descent);
                    json.WriteNumber("leading", font.Leading);
                    json.WriteEndObject();
                }
                json.WriteEndArray();
                WriteOptional(json, "validNameChars", config.ValidNameChars);
                WriteOptional(json, "validBookChars", config.ValidBookChars);
                json.WriteEndObject();
            }
            return buffer.ToArray();
        }

        private static void WriteOptional(Utf8JsonWriter json, string name, string? value)
        {
            if (value is null)
                json.WriteNull(name);
            else
                json.WriteString(name, value);
        }
    }
}
