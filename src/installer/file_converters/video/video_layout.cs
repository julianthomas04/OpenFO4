namespace AssetConverter.Video
{
    /// <summary>
    /// Which game files are videos and where their conversions go: every <c>.bk2</c> directly in the game's <c>Data\Video</c> folder becomes
    /// its lowercased base name with the extension <c>.mp4</c> below <see cref="InstallLayout.VideoFolder"/> (<c>MainMenuLoop.bk2</c> becomes
    /// <c>mainmenuloop.mp4</c>). The engine derives the same name.
    /// </summary>
    internal static class VideoLayout
    {
        public const string SourceFolder = "Video";
        public const string SourceExtension = ".bk2";
        public const string Mp4Extension = ".mp4";

        /// <summary>
        /// The ISO 639-2 language of the audio tracks of a video, by track index, as verified by listening to the game's videos: English, French, Italian, German, Spanish.
        /// A track beyond the table is <see cref="UndeterminedLanguage"/>.
        /// </summary>
        public static readonly IReadOnlyList<string> AudioLanguages = ["eng", "fra", "ita", "deu", "spa"];

        public const string UndeterminedLanguage = "und";

        public static string LanguageOfTrack(int index) => LanguageOfTrack(index, AudioLanguages);

        public static string LanguageOfTrack(int index, IReadOnlyList<string> languages) => index < languages.Count ? languages[index] : UndeterminedLanguage;

        public static string SourceDirectory(string gameDataDirectory) => Path.Combine(gameDataDirectory, SourceFolder);

        public static string OutputPath(string videoRoot, string sourcePath) =>
            Path.Combine(videoRoot, Path.GetFileNameWithoutExtension(sourcePath).ToLowerInvariant() + Mp4Extension);
    }
}
