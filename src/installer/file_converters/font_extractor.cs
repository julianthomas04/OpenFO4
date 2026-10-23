using AssetConverter.Pipeline;
using AssetConverter.UiLayout;

namespace AssetConverter
{
    public sealed class FontExtractionException(string message, Exception? inner = null) : ConversionStepException(message, inner);

    /// Extracts Fallout 4's embedded UI fonts (Roboto Condensed, Share Tech
    /// Mono, ...) from every font library the installed language's font
    /// configuration lists (`Interface\fonts_<lang>.swf`, `fonts_console.swf`, ...)
    /// in `Fallout4 - Interface.ba2`, then exports each embedded font to an
    /// open-format TTF via the vendored JPEXS ffdec, into the install's
    /// `assets\interface\fonts`. The engine loads every .ttf there and references
    /// fonts by family name. Skipped when the folder's sidecar records the current
    /// <see cref="ConversionStamp"/>: this extractor's version, the language, the
    /// font configuration and every library.
    ///
    /// The font is REQUIRED — the engine hard-fails to launch without it — so
    /// every failure THROWS (fails the install loudly) rather than leaving a
    /// fontless, non-runnable install. The export is published by a single
    /// same-volume directory rename from a sibling temp, so the fonts dir only
    /// ever holds a complete set; a crash mid-publish leaves NO fonts (never a
    /// partial set), and the next run re-extracts.
    public static class FontExtractor
    {
        /// <summary>Raised whenever the extraction changes its output for the same game files.</summary>
        internal const int ExtractorVersion = 1;

        /// <returns><c>true</c> when the fonts were extracted, <c>false</c> when they were already present.</returns>
        public static bool Extract(string fo4Location, string settingsDirectory, string stagingRoot, string fontsOut, MemoryBudget memory, CancellationToken ct = default)
        {
            string language = GameLanguage.Resolve(fo4Location, settingsDirectory);
            (SourceFingerprint config, IReadOnlyList<string> libraryNames) = FontLibraries(fo4Location, language);
            var libraries = InterfaceArchive.ReadMatching(fo4Location, path => libraryNames.Contains(path, StringComparer.OrdinalIgnoreCase))
                .OrderBy(library => libraryNames.ToList().FindIndex(name => string.Equals(name, library.Path, StringComparison.OrdinalIgnoreCase)))
                .ToList();
            if (libraries.Count != libraryNames.Count)
                throw new FontExtractionException($"Font extraction: the interface archive holds {libraries.Count} of the font libraries {string.Join(", ", libraryNames)}.");
            var stamp = new ConversionStamp(ExtractorVersion, language, [config, .. libraries.Select(l => SourceFingerprint.Of(l.Path, l.Content))], Ffdec.Version(memory, ct));
            string? reason = stamp.SidecarStaleReason(fontsOut);
            if (reason is null)
            {
                Console.WriteLine($"Fonts of {string.Join(", ", libraryNames)} already present in {fontsOut}; skipping.");
                return false;
            }
            Console.WriteLine($"Fonts in {fontsOut} are extracted again: {reason}.");
            int count = 0;
            AtomicPublish.Folder(fontsOut, tmp =>
            {
                foreach ((string library, byte[] content) in libraries)
                    ExportLibrary(library, content, stagingRoot, tmp, memory, ct);

                count = Directory.GetFiles(tmp, "*.ttf").Length;
                if (count == 0)
                    throw new FontExtractionException($"Font extraction: ffdec produced no TTFs from {string.Join(", ", libraryNames)}.");
                stamp.WriteSidecar(tmp);
            });
            Console.WriteLine($"Extracted {count} FO4 UI font(s) from {string.Join(", ", libraryNames)} -> {fontsOut}");
            return true;
        }

        /// <summary>The installed language's font configuration and the font libraries it lists, in its order.</summary>
        static (SourceFingerprint Config, IReadOnlyList<string> Libraries) FontLibraries(string fo4Location, string language)
        {
            var entries = InterfaceArchive.ReadMatching(fo4Location, path =>
                string.Equals(path, UiLayoutExtractor.LanguageFontConfigEntry(language), StringComparison.OrdinalIgnoreCase)
                || string.Equals(path, UiLayoutExtractor.DefaultFontConfigEntry, StringComparison.OrdinalIgnoreCase));
            (string configPath, byte[] content) = UiLayoutExtractor.SelectFontConfig(entries, language);
            IReadOnlyList<string> libraries;
            try
            {
                libraries = FontConfig.Parse(GameText.Decode(content)).FontLibraries;
            }
            catch (FontConfigException e)
            {
                throw new FontExtractionException($"Font extraction: '{configPath}' cannot be read: {e.Message}", e);
            }
            return libraries.Count > 0
                ? (SourceFingerprint.Of(configPath, content), libraries)
                : throw new FontExtractionException($"Font extraction: '{configPath}' lists no font library.");
        }

        /// <summary>Exports one library's fonts into its own folder first, so two libraries can never overwrite each other's files.</summary>
        static void ExportLibrary(string library, byte[] content, string stagingRoot, string tmp, MemoryBudget memory, CancellationToken ct)
        {
            string libraryDir = Path.Combine(tmp, Path.GetFileNameWithoutExtension(library));
            string swfPath = Path.Combine(stagingRoot, library);
            try
            {
                Directory.CreateDirectory(Path.GetDirectoryName(swfPath)!);
                File.WriteAllBytes(swfPath, content);
                Directory.CreateDirectory(libraryDir);
                Ffdec.RunExporting(memory, libraryDir, ["-export", "font", libraryDir, swfPath], ct);
                foreach (string ttf in Directory.GetFiles(libraryDir, "*.ttf"))
                {
                    TtfNormalizer.NormalizeFile(ttf);
                    string published = Path.Combine(tmp, Path.GetFileName(ttf));
                    if (File.Exists(published))
                        throw new FontExtractionException($"Font extraction: '{library}' and an earlier library both export '{Path.GetFileName(ttf)}'.");
                    File.Move(ttf, published);
                }
            }
            finally
            {
                Scratch.TryDeleteFile(swfPath);
                Scratch.TryDeleteDir(libraryDir);
            }
        }
    }
}
