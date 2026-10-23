namespace AssetConverter.UiLayout
{
    internal sealed class GameLanguageException(string message) : ConversionStepException(message);

    /// <summary>
    /// The language the game is installed in: <c>sLanguage</c> in the game's INI cascade (shipped defaults first, the user's files after,
    /// the last statement winning), read by the engine's INI rules: the name matches case-insensitively in any section (also as
    /// <c>sLanguage:General</c>), lines starting with <c>;</c> are comments, a value ends at its first <c>;</c>, and a leading byte order
    /// mark is ignored. The code is used in lowercase. It names the localized files: <c>Translate_&lt;lang&gt;.txt</c>,
    /// <c>Fallout4_&lt;lang&gt;.STRINGS</c>, <c>FontConfig_&lt;lang&gt;.txt</c>.
    /// </summary>
    internal static class GameLanguage
    {
        private const string Key = "sLanguage";
        private const char Comment = ';';
        private const char SectionQualifier = ':';

        public static string Resolve(string fo4Location, string settingsDirectory) =>
            FromFiles(GameIniFiles.Existing(GameIniFiles.Sources(fo4Location, settingsDirectory)).ToList());

        /// <param name="iniPaths">The INIs in cascade order, lowest precedence first.</param>
        public static string FromFiles(IReadOnlyList<string> iniPaths)
        {
            string? language = null;
            string? from = null;
            foreach (string path in iniPaths)
            {
                if (FromText(File.ReadAllText(path)) is { } stated)
                {
                    language = stated;
                    from = path;
                }
            }
            if (language is null)
                throw new GameLanguageException($"None of the game's INIs ({string.Join(", ", iniPaths)}) states {Key}, so the installed language is unknown.");
            if (language.Length == 0 || !language.All(char.IsAsciiLetter))
                throw new GameLanguageException($"{Key}={language} in '{from}' is not a language code of letters.");
            return language.ToLowerInvariant();
        }

        /// <returns>The last <c>sLanguage</c> the text states, or <c>null</c> when it states none.</returns>
        public static string? FromText(string iniText)
        {
            string? value = null;
            foreach (string rawLine in iniText.TrimStart('\uFEFF').Split('\n'))
            {
                string line = rawLine.Trim();
                if (line.Length == 0 || line[0] is Comment or '[')
                    continue;
                int equals = line.IndexOf('=', StringComparison.Ordinal);
                if (equals < 0)
                    continue;
                string name = line[..equals].Split(SectionQualifier)[0].Trim();
                if (string.Equals(name, Key, StringComparison.OrdinalIgnoreCase))
                    value = line[(equals + 1)..].Split(Comment)[0].Trim();
            }
            return value;
        }
    }
}
