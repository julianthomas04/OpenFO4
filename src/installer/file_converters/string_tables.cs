using Mutagen.Bethesda.Strings;

namespace AssetConverter
{
    /// <summary>The string tables of one language below a folder, as the ESM reader resolves a localized plugin's names from them.</summary>
    /// <param name="Language">The game's language code (<c>sLanguage</c>), e.g. <c>en</c>, <c>de</c>, <c>ptbr</c>.</param>
    public sealed record StringTableSet(string Folder, string Language)
    {
        /// <summary>The language as the plugin reader names it.</summary>
        /// <exception cref="InvalidOperationException">The code is not one of the game's languages.</exception>
        public Language PluginLanguage =>
            StringsUtility.TryGetIsoLanguage(Language, out Language language)
                ? language
                : throw new InvalidOperationException($"'{Language}' is not a language the plugin's string tables can be read in.");
    }

    /// <summary>
    /// Fallout4.esm is a localized plugin: record names and descriptions are ids into the <c>Fallout4_&lt;lang&gt;</c> string tables
    /// that ship inside <c>Fallout4 - Interface.ba2</c>. Extracting the installed language's tables next to the staged ESM lets the ESM
    /// reader resolve the ids.
    /// </summary>
    public static class StringTables
    {
        /// <summary>Raised whenever the extraction changes its output for the same game files.</summary>
        internal const int ExtractorVersion = 1;

        public const string FolderName = "STRINGS";
        static readonly string[] Extensions = [".STRINGS", ".DLSTRINGS", ".ILSTRINGS"];

        public static string Directory(string stagingRoot) => Path.Combine(stagingRoot, FolderName);

        /// <summary>Extracts the tables of <paramref name="language"/> below <paramref name="stagingRoot"/> unless they are already there as the archive holds them.</summary>
        public static StringTableSet Extract(string fo4Location, string stagingRoot, string language)
        {
            var tables = new StringTableSet(Directory(stagingRoot), language);
            string prefix = $@"{FolderName}\{BaseName(language)}.";
            var read = InterfaceArchive.ReadMatching(fo4Location, archivePath => archivePath.StartsWith(prefix, StringComparison.OrdinalIgnoreCase));
            var stamp = new ConversionStamp(ExtractorVersion, language, [.. read.OrderBy(t => t.Path, StringComparer.OrdinalIgnoreCase).Select(t => SourceFingerprint.Of(t.Path, t.Content))]);
            string stampedFile = TableFile(tables, Extensions[0]);
            string? reason = stamp.SidecarStaleReason(stampedFile);
            if (reason is null && Extensions.All(extension => File.Exists(TableFile(tables, extension))))
                return tables;

            System.IO.Directory.CreateDirectory(tables.Folder);
            foreach ((string path, byte[] content) in read)
                AtomicPublish.File(Path.Combine(stagingRoot, path), content);
            Require(tables, $"'Fallout4 - Interface.ba2' in '{fo4Location}'");
            stamp.WriteSidecar(stampedFile);
            Console.WriteLine($"Extracted {read.Count} string table(s) to {tables.Folder}" + (reason is null ? "." : $" ({reason})."));
            return tables;
        }

        /// <summary>Throws when any table of the set's language is missing from its folder.</summary>
        public static void Require(StringTableSet tables, string expectedSource)
        {
            var missing = Extensions.Where(extension => !File.Exists(TableFile(tables, extension))).ToList();
            if (missing.Count > 0)
                throw new FileNotFoundException(
                    $"The '{tables.Language}' string table(s) {string.Join(", ", missing.Select(e => BaseName(tables.Language) + e))} are missing from '{tables.Folder}'; expected from {expectedSource}.");
        }

        static string BaseName(string language) => $"Fallout4_{language}";

        static string TableFile(StringTableSet tables, string extension) => Path.Combine(tables.Folder, BaseName(tables.Language) + extension);
    }
}
