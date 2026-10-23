using AssetConverter.Swf;

namespace AssetConverter.UiLayout
{
    /// <summary>
    /// The vertical metrics of a font a font configuration maps a class to, as the font library embeds it: <paramref name="Ascent"/>,
    /// <paramref name="Descent"/> and <paramref name="Leading"/> in font units, <paramref name="EmSize"/> of them to the em.
    /// </summary>
    internal sealed record FontMetrics(string Family, string Style, string Library, int EmSize, int Ascent, int Descent, int Leading)
    {
        private static readonly Dictionary<string, (bool Bold, bool Italic)> Styles = new(StringComparer.Ordinal)
        {
            ["Normal"] = (false, false),
            ["Bold"] = (true, false),
            ["Italic"] = (false, true),
            ["BoldItalic"] = (true, true),
        };

        /// <summary>The metrics of every family and style the configuration maps, ordered by family and style.</summary>
        /// <param name="libraries">The configuration's font libraries, by the path the configuration names them with.</param>
        /// <exception cref="FontConfigException">A style is unknown, or no library, or libraries with different metrics, embed a mapped font with its metrics.</exception>
        public static IReadOnlyList<FontMetrics> Of(FontConfig config, IReadOnlyDictionary<string, SwfMovie> libraries)
        {
            var fonts = new List<FontMetrics>();
            foreach ((string family, string style) in config.Maps.Select(m => (m.Family, m.Style)).Distinct()
                .OrderBy(f => f.Family, StringComparer.Ordinal).ThenBy(f => f.Style, StringComparer.Ordinal))
            {
                if (!Styles.TryGetValue(style, out var flags))
                    throw new FontConfigException($"The font configuration maps '{family}' with the style '{style}', which is none of {string.Join(", ", Styles.Keys)}.");
                var embedded = config.FontLibraries
                    .SelectMany(library => LibraryMovie(libraries, library).Characters.Values.OfType<SwfFont>()
                        .Where(font => font.Name == family && font.Bold == flags.Bold && font.Italic == flags.Italic)
                        .Select(font => Metrics(font, style, library)))
                    .ToList();
                if (embedded.Count == 0)
                    throw new FontConfigException($"No font library of the configuration embeds '{family}' {style}.");
                if (embedded.Select(f => f with { Library = "" }).Distinct().Count() > 1)
                    throw new FontConfigException($"The font libraries embed '{family}' {style} {embedded.Count} times with different metrics.");
                fonts.Add(embedded[0]);
            }
            return fonts;
        }

        private static SwfMovie LibraryMovie(IReadOnlyDictionary<string, SwfMovie> libraries, string library) =>
            libraries.TryGetValue(library, out SwfMovie? movie)
                ? movie
                : throw new FontConfigException($"The font library '{library}' the configuration names is not in the interface archive.");

        private static FontMetrics Metrics(SwfFont font, string style, string library) =>
            font is { Ascent: { } ascent, Descent: { } descent, Leading: { } leading }
                ? new FontMetrics(font.Name, style, library, font.EmSize, ascent, descent, leading)
                : throw new FontConfigException($"'{font.Name}' {style} in '{library}' carries no layout metrics.");
    }
}
