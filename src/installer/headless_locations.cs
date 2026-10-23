using AssetConverter.Pipeline;

namespace AssetConverter
{
    /// <summary>The game and install folders of a headless install: the ones it is told, else the development defaults, which only a repository checkout has.</summary>
    internal static class HeadlessLocations
    {
        /// <param name="gameFolderFromEnvironment">The game folder named by <c>OPENFO4_FO4_DIR</c>, if any.</param>
        /// <param name="devGameFolder">The game folder to assume when none is named; null where there is none.</param>
        /// <param name="devInstallFolder">The install folder to assume when none is named; null where there is none.</param>
        /// <exception cref="LaunchArgumentException">A folder is named nowhere and has no default.</exception>
        public static InstallLocations Resolve(LaunchArguments launch, string? gameFolderFromEnvironment, string? devGameFolder, string? devInstallFolder)
        {
            string? game = launch.GameDirectory ?? gameFolderFromEnvironment ?? devGameFolder;
            string? install = launch.InstallDirectory ?? devInstallFolder;
            if (game == null)
                throw new LaunchArgumentException($"A headless install needs the Fallout 4 folder: pass {LaunchArguments.GameDirectoryFlag} <folder> (or set OPENFO4_FO4_DIR).");
            if (install == null)
                throw new LaunchArgumentException($"A headless install needs the folder to install into: pass {LaunchArguments.InstallDirectoryFlag} <folder>.");
            return new InstallLocations(Full(game), Full(install), launch.SettingsDirectory is { } settings ? Full(settings) : null);
        }

        private static string Full(string folder)
        {
            try
            {
                return InstallFolderRules.FullPath(folder);
            }
            catch (Exception ex) when (ex is ArgumentException or NotSupportedException or IOException)
            {
                throw new LaunchArgumentException($"'{folder}' is not a valid folder path: {ex.Message}");
            }
        }
    }
}
