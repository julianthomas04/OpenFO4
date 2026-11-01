using System.Text.Json;
using AssetConverter.Abc;
using AssetConverter.Swf;

namespace AssetConverter.UiLayout
{
    internal sealed class UiLayoutException(string message, Exception? inner = null) : ConversionStepException(message, inner);

    /// <summary>
    /// Converts the game's interface movies, the installed language's translate table and its font configuration into the layout documents
    /// below <c>assets\interface\layout</c>: one <c>&lt;Movie&gt;.json</c> per movie (top-level <c>Interface\*.swf</c> except the font
    /// libraries, and the condition clips), <c>translate.json</c> and <c>fontconfig.json</c> (with the metrics of the fonts its libraries
    /// embed). A document is rewritten only when it is not a current conversion of its archive entries (see <see cref="UiDocumentHeader"/>);
    /// documents no longer produced are deleted. Every failure fails the step, naming the archive entry it happened in.
    /// </summary>
    internal static class UiLayoutExtractor
    {
        private const string InterfacePrefix = @"Interface\";
        private const string ConditionClipsPrefix = @"Interface\Components\ConditionClips\";
        private const string MovieExtension = ".swf";
        private const string FontLibraryPrefix = "fonts_";
        private const string JsonExtension = ".json";

        /// <returns><c>true</c> when there was work: a document was written or an obsolete one deleted.</returns>
        public static bool Extract(string fo4Location, string settingsDirectory, string outDir)
        {
            (string language, var entries) = ReadEntries(fo4Location, settingsDirectory);
            return Convert(entries, language, outDir);
        }

        const string PendingDetail = "UI layout";

        /// <summary>Reads the archive and works out which documents are out of date or no longer produced; null when the folder is current.</summary>
        internal static PendingExtraction? Plan(string fo4Location, string settingsDirectory, string outDir) =>
            Plan(() => ReadEntries(fo4Location, settingsDirectory), outDir);

        /// <param name="read">Reads the language and the archive entries; called again when the extraction runs, so the plan keeps none of them.</param>
        internal static PendingExtraction? Plan(Func<(string Language, List<(string Path, byte[] Content)> Entries)> read, string outDir)
        {
            (string language, var entries) = read();
            var plan = PlanConversion(entries, language, outDir);
            if (!plan.HasWork)
            {
                ReportSummary(plan);
                return null;
            }
            return new PendingExtraction(PendingDetail, () =>
            {
                (string runLanguage, var runEntries) = read();
                Convert(runEntries, runLanguage, outDir);
            });
        }

        private static (string Language, List<(string Path, byte[] Content)> Entries) ReadEntries(string fo4Location, string settingsDirectory)
        {
            string language = GameLanguage.Resolve(fo4Location, settingsDirectory);
            var entries = InterfaceArchive.ReadMatching(fo4Location, path => IsMovie(path) || IsLanguageFile(path, language)).ToList();
            (string configPath, byte[] config) = SelectFontConfig(entries, language);
            IReadOnlyList<string> libraries = Guard(configPath, () => FontConfig.Parse(GameText.Decode(config))).FontLibraries;
            entries.AddRange(InterfaceArchive.ReadMatching(fo4Location, path => libraries.Contains(path, StringComparer.OrdinalIgnoreCase)));
            return (language, entries);
        }

        public static bool IsMovie(string archivePath)
        {
            if (!archivePath.EndsWith(MovieExtension, StringComparison.OrdinalIgnoreCase))
                return false;
            if (archivePath.StartsWith(ConditionClipsPrefix, StringComparison.OrdinalIgnoreCase))
                return !archivePath[ConditionClipsPrefix.Length..].Contains('\\', StringComparison.Ordinal);
            if (!archivePath.StartsWith(InterfacePrefix, StringComparison.OrdinalIgnoreCase))
                return false;
            string name = archivePath[InterfacePrefix.Length..];
            return !name.Contains('\\', StringComparison.Ordinal) && !name.StartsWith(FontLibraryPrefix, StringComparison.OrdinalIgnoreCase);
        }

        public static string TranslateEntry(string language) => $@"{InterfacePrefix}Translate_{language}.txt";

        public static string LanguageFontConfigEntry(string language) => $@"{InterfacePrefix}FontConfig_{language}.txt";

        public const string DefaultFontConfigEntry = @"Interface\FontConfig.txt";

        private static bool IsLanguageFile(string archivePath, string language) =>
            new[] { TranslateEntry(language), LanguageFontConfigEntry(language), DefaultFontConfigEntry }
                .Any(entry => string.Equals(entry, archivePath, StringComparison.OrdinalIgnoreCase));

        /// <summary>The document a movie is written to, relative to the layout folder: <c>Pipboy_InvPage.json</c>, <c>Components/ConditionClips/Condition_Head.json</c>.</summary>
        public static string MovieName(string archivePath) =>
            Path.ChangeExtension(archivePath[InterfacePrefix.Length..], null).Replace('\\', '/');

        /// <summary>
        /// The font configuration of a language: its own <c>FontConfig_&lt;lang&gt;.txt</c> when the game ships one, else the default
        /// <c>FontConfig.txt</c>.
        /// </summary>
        public static (string Path, byte[] Content) SelectFontConfig(IReadOnlyList<(string Path, byte[] Content)> entries, string language) =>
            Find(entries, LanguageFontConfigEntry(language)) ?? Find(entries, DefaultFontConfigEntry)
                ?? throw new UiLayoutException($"The interface archive has neither '{LanguageFontConfigEntry(language)}' nor '{DefaultFontConfigEntry}'.");

        private static (string Path, byte[] Content)? Find(IReadOnlyList<(string Path, byte[] Content)> entries, string archivePath) =>
            entries.Where(e => string.Equals(e.Path, archivePath, StringComparison.OrdinalIgnoreCase)).Select(e => ((string, byte[])?)e).FirstOrDefault();

        internal static bool Convert(IReadOnlyList<(string Path, byte[] Content)> entries, string language, string outDir) =>
            Run(PlanConversion(entries, language, outDir));

        private sealed record ConversionPlan(string OutDir, string Language, IReadOnlyList<PendingDocument> Documents, IReadOnlyList<PendingDocument> Stale, IReadOnlyList<string> Unproduced)
        {
            public bool HasWork => Stale.Count > 0 || Unproduced.Count > 0;
        }

        private static ConversionPlan PlanConversion(IReadOnlyList<(string Path, byte[] Content)> entries, string language, string outDir)
        {
            var documents = new List<PendingDocument>();
            (string translatePath, byte[] translate) = Find(entries, TranslateEntry(language))
                ?? throw new UiLayoutException($"The interface archive has no '{TranslateEntry(language)}' for the installed language '{language}'.");
            documents.Add(new PendingDocument(InstallLayout.TranslateFileName, UiDocumentHeader.TranslateKind,
                UiDocumentHeader.Stamp(language, SourceFingerprint.Of(translatePath, translate)),
                stamp => Guard(translatePath, () => TranslateTable.Parse(translate, translatePath).ToJson(language, stamp))));
            documents.Add(FontConfigDocumentOf(entries, language));

            foreach ((string path, byte[] content) in entries.Where(e => IsMovie(e.Path)))
            {
                string movie = MovieName(path);
                documents.Add(new PendingDocument(movie + JsonExtension, UiDocumentHeader.MovieKind, UiDocumentHeader.Stamp(null, SourceFingerprint.Of(path, content)),
                    stamp => Guard(path, () => ConvertMovie(movie, stamp, content))));
            }
            RejectCollisions(documents);

            var stale = documents.Where(d => UiDocumentHeader.StaleReason(Path.Combine(outDir, d.RelativePath), d.Kind, d.Stamp) is not null).ToList();
            return new ConversionPlan(outDir, language, documents, stale, FindUnproduced(outDir, documents.Select(d => d.RelativePath)));
        }

        /// <returns><c>true</c> when there was work: a document was written or an obsolete one deleted.</returns>
        private static bool Run(ConversionPlan plan)
        {
            Directory.CreateDirectory(plan.OutDir);
            ConversionGuard.RunAll(
                plan.Stale.Select(document => (Action)(() => AtomicPublish.File(Path.Combine(plan.OutDir, document.RelativePath), document.Convert(document.Stamp)))),
                Environment.ProcessorCount,
                failures => new UiLayoutException(ConversionGuard.Describe(failures), failures[0]));

            DeleteUnproduced(plan.Unproduced);
            ReportSummary(plan);
            return plan.HasWork;
        }

        private static void ReportSummary(ConversionPlan plan) =>
            Console.WriteLine($"UI layout: {plan.Stale.Count} of {plan.Documents.Count} document(s) converted, the rest current, in {plan.OutDir} (language '{plan.Language}').");

        private sealed record PendingDocument(string RelativePath, string Kind, ConversionStamp Stamp, Func<ConversionStamp, byte[]> Convert);

        /// <summary>The font configuration document: converted from the configuration and every font library it names, whose fonts it measures.</summary>
        private static PendingDocument FontConfigDocumentOf(IReadOnlyList<(string Path, byte[] Content)> entries, string language)
        {
            (string configPath, byte[] config) = SelectFontConfig(entries, language);
            FontConfig fontConfig = Guard(configPath, () => FontConfig.Parse(GameText.Decode(config)));
            var libraries = fontConfig.FontLibraries
                .Select(library => (Name: library, Entry: Find(entries, library)
                    ?? throw new UiLayoutException($"'{configPath}' names the font library '{library}', which is not in the interface archive.")))
                .ToList();
            var sources = libraries.Select(library => SourceFingerprint.Of(library.Entry.Path, library.Entry.Content)).Prepend(SourceFingerprint.Of(configPath, config));
            return new PendingDocument(InstallLayout.FontConfigFileName, UiDocumentHeader.FontConfigKind, UiDocumentHeader.Stamp(language, [.. sources]), stamp =>
            {
                var movies = new Dictionary<string, SwfMovie>(StringComparer.OrdinalIgnoreCase);
                foreach ((string name, (string path, byte[] content)) in libraries)
                    movies[name] = Guard(path, () => SwfMovieReader.Read(content));
                return Guard(configPath, () => FontConfigDocument.Write(fontConfig, FontMetrics.Of(fontConfig, movies), language, stamp));
            });
        }

        private static byte[] ConvertMovie(string movie, ConversionStamp stamp, byte[] content)
        {
            SwfMovie parsed = SwfMovieReader.Read(content);
            return UiMovieDocument.Write(movie, stamp, parsed, AbcConstantExtractor.Extract(parsed, movie));
        }

        private static T Guard<T>(string archivePath, Func<T> convert) =>
            ConversionGuard.Run(archivePath, convert, (message, cause) => new UiLayoutException(message, cause));

        private static void RejectCollisions(List<PendingDocument> documents)
        {
            var collision = documents.GroupBy(d => d.RelativePath, StringComparer.OrdinalIgnoreCase).FirstOrDefault(g => g.Count() > 1);
            if (collision is not null)
                throw new UiLayoutException($"Two documents would be written to '{collision.Key}': {string.Join(", ", collision.Select(d => d.Stamp.Sources[0].Entry))}.");
        }

        private static readonly string[] DocumentKinds = [UiDocumentHeader.MovieKind, UiDocumentHeader.TranslateKind, UiDocumentHeader.FontConfigKind];

        /// <summary>The documents this step wrote for game files that no longer produce one, and its leftover temporary files; any other file stays.</summary>
        private static List<string> FindUnproduced(string outDir, IEnumerable<string> produced)
        {
            if (!Directory.Exists(outDir)) return [];
            var keep = produced.Select(p => Path.GetFullPath(Path.Combine(outDir, p))).ToHashSet(StringComparer.OrdinalIgnoreCase);
            return [.. Directory.EnumerateFiles(outDir, "*", SearchOption.AllDirectories).Where(file =>
            {
                bool leftover = file.EndsWith(JsonExtension + AtomicPublish.TemporarySuffix, StringComparison.OrdinalIgnoreCase);
                bool unproducedDocument = file.EndsWith(JsonExtension, StringComparison.OrdinalIgnoreCase)
                    && !keep.Contains(Path.GetFullPath(file)) && IsOwnDocument(file);
                return leftover || unproducedDocument;
            })];
        }

        private static void DeleteUnproduced(IEnumerable<string> files)
        {
            foreach (string file in files)
            {
                File.Delete(file);
                Console.WriteLine($"UI layout: removed '{file}', which no game file produces any more.");
            }
        }

        /// <summary>Whether a file is a document of this step: a JSON object with a conversion record and one of its kinds.</summary>
        private static bool IsOwnDocument(string file)
        {
            try
            {
                using var stream = File.OpenRead(file);
                using JsonDocument document = JsonDocument.Parse(stream);
                return document.RootElement.ValueKind == JsonValueKind.Object
                    && document.RootElement.TryGetProperty(ConversionStamp.Property, out _)
                    && document.RootElement.TryGetProperty("kind", out JsonElement kind) && kind.ValueKind == JsonValueKind.String
                    && DocumentKinds.Contains(kind.GetString(), StringComparer.Ordinal);
            }
            catch (JsonException)
            {
                return false;
            }
        }
    }
}
