namespace AssetConverter
{
    /// <summary>
    /// The colour lookup tables image spaces name by their path below the textures folder (<c>Effects\LUTs\LUT_CW_ClearDAY.DDS</c>). The game
    /// keeps them in the general archive Misc.ba2, which the texture archives' extraction never reads.
    /// </summary>
    internal static class ColourLuts
    {
        public const string ArchiveName = "Fallout4 - Misc.ba2";
        public const string ArchiveFolder = @"Textures\Effects\LUTs\";
        private const string Extension = ".dds";

        /// <summary>Whether the archive entry <paramref name="archivePath"/> is a colour LUT.</summary>
        public static bool IsColourLut(string archivePath) => ArchivePath.Matches(archivePath, ArchiveFolder, Extension);
    }
}
