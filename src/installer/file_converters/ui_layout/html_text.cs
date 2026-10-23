using System.Net;
using System.Text.RegularExpressions;

namespace AssetConverter.UiLayout
{
    /// <summary>
    /// A run of an HTML text field's initial text and the format in force over it: every <c>TAG.ATTRIBUTE</c> of the enclosing tags (the
    /// innermost wins) plus <c>B</c>, <c>I</c> and <c>U</c> for the style tags.
    /// </summary>
    internal sealed record HtmlTextRun(string Text, IReadOnlyDictionary<string, string> Format);

    /// <summary>
    /// The Flash HTML subset of a text field's initial text: tags with attributes in either quote style or unquoted, character entities, paragraphs
    /// (<c>P</c>, separated by a line break) and <c>BR</c>. <see cref="Attributes"/> keeps the first value of each <c>TAG.ATTRIBUTE</c> in
    /// document order, including tags that enclose no text.
    /// </summary>
    internal sealed partial record HtmlText(IReadOnlyList<HtmlTextRun> Runs, IReadOnlyDictionary<string, string> Attributes)
    {
        private const string Paragraph = "P";
        private const string LineBreak = "BR";
        public const string StyleOn = "true";
        private static readonly HashSet<string> StyleTags = new(StringComparer.Ordinal) { "B", "I", "U" };

        [GeneratedRegex(@"<(?<close>/)?(?<tag>[A-Za-z]+)(?<attributes>(?:""[^""]*""|'[^']*'|[^>""'])*?)(?<self>/)?>", RegexOptions.CultureInvariant)]
        private static partial Regex Tag();

        [GeneratedRegex(@"(?<name>[A-Za-z]+)\s*=\s*(?:""(?<value>[^""]*)""|'(?<value>[^']*)'|(?<value>[^\s""'>]+))", RegexOptions.CultureInvariant)]
        private static partial Regex Attribute();

        public static HtmlText Parse(string html)
        {
            var runs = new List<HtmlTextRun>();
            var attributes = new Dictionary<string, string>(StringComparer.Ordinal);
            var open = new List<(string Tag, Dictionary<string, string> Format)>();
            bool anyParagraph = false;
            int position = 0;

            void AddText(string text)
            {
                if (text.Length == 0)
                    return;
                var format = new SortedDictionary<string, string>(StringComparer.Ordinal);
                foreach ((_, Dictionary<string, string> tagFormat) in open)
                    foreach ((string key, string value) in tagFormat)
                        format[key] = value;
                if (runs.Count > 0 && runs[^1].Format.SequenceEqual(format))
                    runs[^1] = runs[^1] with { Text = runs[^1].Text + text };
                else
                    runs.Add(new HtmlTextRun(text, format));
            }

            foreach (Match tag in Tag().Matches(html))
            {
                AddText(WebUtility.HtmlDecode(html[position..tag.Index]));
                position = tag.Index + tag.Length;
                string name = tag.Groups["tag"].Value.ToUpperInvariant();
                if (tag.Groups["close"].Success)
                {
                    int index = open.FindLastIndex(o => o.Tag == name);
                    if (index >= 0)
                        open.RemoveRange(index, open.Count - index);
                    continue;
                }
                if (name == LineBreak)
                {
                    AddText("\n");
                    continue;
                }
                if (name == Paragraph)
                {
                    if (anyParagraph)
                        AddText("\n");
                    anyParagraph = true;
                }
                var format = new Dictionary<string, string>(StringComparer.Ordinal);
                if (StyleTags.Contains(name))
                    format[name] = StyleOn;
                foreach (Match attribute in Attribute().Matches(tag.Groups["attributes"].Value))
                {
                    string key = $"{name}.{attribute.Groups["name"].Value.ToUpperInvariant()}";
                    string value = WebUtility.HtmlDecode(attribute.Groups["value"].Value);
                    format[key] = value;
                    attributes.TryAdd(key, value);
                }
                if (!tag.Groups["self"].Success)
                    open.Add((name, format));
            }
            AddText(WebUtility.HtmlDecode(html[position..]));
            return new HtmlText(runs, attributes);
        }
    }
}
