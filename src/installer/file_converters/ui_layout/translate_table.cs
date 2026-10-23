using System.Text;
using System.Text.Json;

namespace AssetConverter.UiLayout
{
    internal sealed class TranslateTableException(string message) : Exception(message);

    /// <summary>
    /// The game's UI string table (<c>Interface\Translate_&lt;lang&gt;.txt</c>, UTF-16 with a byte order mark): one <c>$KEY</c> per
    /// line, a tab, then the text. Tabs between key and text and at the end of the text are not part of the text. Keys are case-sensitive.
    /// A key stated twice with different texts is left unresolved rather than picking one.
    /// </summary>
    internal sealed record TranslateTable(IReadOnlyDictionary<string, string> Entries, IReadOnlyDictionary<string, string> Unresolved)
    {
        private const char KeyMarker = '$';

        public static TranslateTable Parse(byte[] file, string where)
        {
            string text;
            using (var reader = new StreamReader(new MemoryStream(file), Encoding.Unicode, detectEncodingFromByteOrderMarks: true))
                text = reader.ReadToEnd();

            var entries = new SortedDictionary<string, string>(StringComparer.Ordinal);
            var conflicts = new SortedDictionary<string, HashSet<string>>(StringComparer.Ordinal);
            string[] lines = text.Split('\n');
            for (int i = 0; i < lines.Length; i++)
            {
                string line = lines[i].TrimEnd('\r');
                if (line.Trim().Length == 0)
                    continue;
                int tab = line.IndexOf('\t', StringComparison.Ordinal);
                if (tab <= 0 || line[0] != KeyMarker)
                    throw new TranslateTableException($"{where} line {i + 1} is not '$KEY<tab>text': '{line}'.");
                string key = line[..tab];
                string value = line[tab..].Trim('\t');
                if (entries.TryGetValue(key, out string? existing) && existing != value)
                    conflicts.TryAdd(key, [existing]);
                if (conflicts.TryGetValue(key, out HashSet<string>? values))
                    values.Add(value);
                entries[key] = value;
            }
            foreach (string key in conflicts.Keys)
                entries.Remove(key);
            var unresolved = conflicts.ToDictionary(c => c.Key, c => $"stated with {c.Value.Count} different texts", StringComparer.Ordinal);
            return new TranslateTable(entries, unresolved);
        }

        public byte[] ToJson(string language, ConversionStamp stamp)
        {
            using var buffer = new MemoryStream();
            using (var json = new Utf8JsonWriter(buffer, UiDocumentHeader.WriterOptions))
            {
                json.WriteStartObject();
                UiDocumentHeader.Write(json, UiDocumentHeader.TranslateKind, stamp);
                json.WriteString("language", language);
                json.WriteStartObject("entries");
                foreach ((string key, string value) in Entries)
                    json.WriteString(key, value);
                json.WriteEndObject();
                json.WriteStartObject("unresolved");
                foreach ((string key, string reason) in Unresolved.OrderBy(p => p.Key, StringComparer.Ordinal))
                    json.WriteString(key, reason);
                json.WriteEndObject();
                json.WriteEndObject();
            }
            return buffer.ToArray();
        }
    }
}
