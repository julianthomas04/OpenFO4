using System.Text.RegularExpressions;

namespace AssetConverter
{
    /// <summary>One <c>map "$Class" = "Family" Style</c> line.</summary>
    internal sealed record FontConfigMap(string FontClass, string Family, string Style);

    /// <summary>
    /// The game's font configuration (<c>Interface\FontConfig.txt</c> or a language's <c>FontConfig_&lt;lang&gt;.txt</c>): the font
    /// libraries to load, which family each Scaleform font class stands for, and the character sets name and book entry accept.
    /// </summary>
    internal sealed partial record FontConfig(IReadOnlyList<string> FontLibraries, IReadOnlyList<FontConfigMap> Maps, string? ValidNameChars, string? ValidBookChars)
    {
        private const string NormalStyle = "Normal";

        private const char Escape = '\\';

        [GeneratedRegex(@"^map\s+""(?<class>(?:\\.|[^""\\])+)""\s*=\s*""(?<family>(?:\\.|[^""\\])+)""\s+(?<style>\w+)$", RegexOptions.CultureInvariant)]
        private static partial Regex MapLine();

        [GeneratedRegex(@"^(?<directive>\w+)\s+""(?<value>(?:\\.|[^""\\])*)""$", RegexOptions.CultureInvariant)]
        private static partial Regex QuotedLine();

        /// <exception cref="FontConfigException">A line is none of fontlib, map, validNameChars or validBookChars.</exception>
        public static FontConfig Parse(string configText)
        {
            var libraries = new List<string>();
            var maps = new List<FontConfigMap>();
            string? validName = null, validBook = null;
            string[] lines = configText.Split('\n');
            for (int i = 0; i < lines.Length; i++)
            {
                string line = lines[i].Trim();
                if (line.Length == 0)
                    continue;
                if (MapLine().Match(line) is { Success: true } map)
                {
                    maps.Add(new FontConfigMap(Unescape(map.Groups["class"].Value), Unescape(map.Groups["family"].Value), map.Groups["style"].Value));
                    continue;
                }
                Match quoted = QuotedLine().Match(line);
                switch (quoted.Success ? quoted.Groups["directive"].Value : "")
                {
                    case "fontlib":
                        libraries.Add(Unescape(quoted.Groups["value"].Value));
                        break;
                    case "validNameChars":
                        validName = Unescape(quoted.Groups["value"].Value);
                        break;
                    case "validBookChars":
                        validBook = Unescape(quoted.Groups["value"].Value);
                        break;
                    default:
                        throw new FontConfigException($"Font configuration line {i + 1} is not a fontlib, map, validNameChars or validBookChars line: '{line}'.");
                }
            }
            return new FontConfig(libraries, maps, validName, validBook);
        }

        /// <summary>
        /// A quoted value as the game reads it: <c>\\</c> stands for a backslash and <c>\"</c> for a quote; any other backslash is itself
        /// (library paths such as <c>Interface\fonts_en.swf</c> carry single backslashes).
        /// </summary>
        internal static string Unescape(string quoted)
        {
            var value = new System.Text.StringBuilder(quoted.Length);
            for (int i = 0; i < quoted.Length; i++)
            {
                if (quoted[i] == Escape && i + 1 < quoted.Length && quoted[i + 1] is Escape or '"')
                    i++;
                value.Append(quoted[i]);
            }
            return value.ToString();
        }

        /// <returns>
        /// The family <paramref name="fontClass"/> is mapped to. A class mapped once is that line's family whatever its style; a class
        /// mapped several times (one line per style) is the family of its <c>Normal</c> line.
        /// </returns>
        /// <exception cref="FontConfigException">The class is mapped to no family, or to several without a single Normal family.</exception>
        public string FamilyOf(string fontClass)
        {
            List<FontConfigMap> lines = Maps.Where(m => m.FontClass == fontClass).ToList();
            if (lines.Count == 0)
                throw new FontConfigException($"The font configuration maps no family to font class '{fontClass}'.");
            if (lines.Count == 1)
                return lines[0].Family;

            List<string> normalFamilies = lines.Where(l => l.Style == NormalStyle).Select(l => l.Family).Distinct().ToList();
            return normalFamilies.Count switch
            {
                1 => normalFamilies[0],
                0 => throw new FontConfigException($"The font configuration maps font class '{fontClass}' {lines.Count} times, none of them as {NormalStyle}."),
                _ => throw new FontConfigException($"The font configuration maps font class '{fontClass}' to {normalFamilies.Count} different families as {NormalStyle}."),
            };
        }

        public static string FamilyOf(string configText, string fontClass) => Parse(configText).FamilyOf(fontClass);
    }
}
