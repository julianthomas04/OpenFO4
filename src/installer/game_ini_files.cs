namespace AssetConverter
{
    /// <summary>
    /// The game INIs the engine reads (lowest precedence first), where they live, and which of them it cannot start
    /// without: <c>Fallout4_Default.ini</c> (ships with the game) and <c>Fallout4Prefs.ini</c> (written by the game's first
    /// launch). <c>Fallout4.ini</c> repeats the shipped defaults and is also written by the first launch;
    /// <c>Fallout4Custom.ini</c> exists only if the user made one.
    /// </summary>
    internal static class GameIniFiles
    {
        internal enum Origin { Shipped, WrittenByFirstLaunch, UserMade }

        internal readonly record struct Source(string Path, Origin Origin, bool Required);

        public static string DocumentsDirectory() =>
            Path.Combine(Environment.GetFolderPath(Environment.SpecialFolder.MyDocuments), "My Games", "Fallout4");

        public static IReadOnlyList<Source> Sources(string fo4Location, string documentsDirectory) =>
        [
            new(Path.Combine(fo4Location, "Fallout4_Default.ini"), Origin.Shipped, Required: true),
            new(Path.Combine(documentsDirectory, "Fallout4.ini"), Origin.WrittenByFirstLaunch, Required: false),
            new(Path.Combine(documentsDirectory, "Fallout4Prefs.ini"), Origin.WrittenByFirstLaunch, Required: true),
            new(Path.Combine(documentsDirectory, "Fallout4Custom.ini"), Origin.UserMade, Required: false),
        ];

        public static IEnumerable<string> MissingRequired(IEnumerable<Source> sources) =>
            sources.Where(source => source.Required && !File.Exists(source.Path)).Select(source => source.Path);

        /// <summary>Why the install cannot copy the required INIs, or null when they all exist; names every missing file and says what to do about it.</summary>
        public static string? MissingError(string fo4Location, string settingsDirectory)
        {
            var missing = Sources(fo4Location, settingsDirectory).Where(source => source.Required && !File.Exists(source.Path)).ToList();
            var messages = new List<string>();
            AddMessage(messages, missing, Origin.Shipped, "The Fallout 4 installation looks incomplete. Verify the game files in Steam (Properties > Installed Files > Verify integrity of game files), then try again.");
            AddMessage(messages, missing, Origin.WrittenByFirstLaunch, "Fallout 4 has not been started yet. Start Fallout 4 once (to the main menu) so it creates its settings files, then try again. If Fallout 4 keeps them somewhere else, point the Fallout 4 settings folder (--settings-dir) at the folder that holds them.");
            return messages.Count == 0 ? null : string.Join(" ", messages);
        }

        private static void AddMessage(List<string> messages, List<Source> missing, Origin origin, string advice)
        {
            var paths = missing.Where(source => source.Origin == origin).Select(source => source.Path).ToList();
            if (paths.Count > 0) messages.Add($"{advice} Missing: {string.Join(", ", paths)}.");
        }

        public static IEnumerable<string> Existing(IEnumerable<Source> sources) =>
            sources.Where(source => File.Exists(source.Path)).Select(source => source.Path);
    }
}
