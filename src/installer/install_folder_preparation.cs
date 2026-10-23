using AssetConverter.Pipeline;

namespace AssetConverter
{
    /// <summary>Gets the install folder ready for an install: checks it against the rules once more, makes sure the installed game is not running and creates the folders.</summary>
    internal static class InstallFolderPreparation
    {
        private const int SharingViolation = 32;
        private const int LockViolation = 33;

        /// <summary>Prepares the folders of <paramref name="locations"/>; returns why that is not possible, or null when the install can start.</summary>
        public static string? Prepare(InstallLocations locations)
        {
            string? rejection = InstallFolderRules.InstallFolderError(locations.InstallRoot, locations.Fo4Location) ?? RunningGameError(locations.GameExecutable);
            if (rejection != null) return rejection;
            try
            {
                locations.CreateOutputDirectories();
                return null;
            }
            catch (Exception ex) when (ex is IOException or UnauthorizedAccessException or NotSupportedException)
            {
                return $"Cannot create the install folder '{locations.InstallRoot}': {ex.Message}";
            }
        }

        /// <summary>
        /// Why the installed game cannot be replaced, or null when it can: a running game holds its executable open, so opening it for
        /// writing exclusively fails. Nothing is written to the file.
        /// </summary>
        internal static string? RunningGameError(string gameExecutable)
        {
            if (!File.Exists(gameExecutable)) return null;
            try
            {
                using var _ = new FileStream(gameExecutable, FileMode.Open, FileAccess.Write, FileShare.None);
                return null;
            }
            catch (IOException ex) when ((ex.HResult & 0xFFFF) is SharingViolation or LockViolation)
            {
                return $"Close OpenFO4 before reinstalling: '{gameExecutable}' is in use.";
            }
            catch (Exception ex) when (ex is IOException or UnauthorizedAccessException)
            {
                return $"The installed game '{gameExecutable}' cannot be replaced: {ex.Message}";
            }
        }
    }
}
