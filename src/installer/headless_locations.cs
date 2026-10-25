using AssetConverter.Pipeline;

namespace AssetConverter
{
    /// <summary>The game and install folders of a headless install: the game folder from <c>--game-dir</c> or <c>OPENFO4_FO4_DIR</c>, the install folder from <c>--install-dir</c> only.</summary>
    internal static class HeadlessLocations
    {
        public const string GameFolderVariable = "OPENFO4_FO4_DIR";
        public const string InstallFolderVariable = "OPENFO4_INSTALL_DIR";

        /// <summary>
        /// A headless install: the game folder is the <c>--game-dir</c> flag, else <see cref="GameFolderVariable"/>; the install folder is the
        /// <c>--install-dir</c> flag only (<see cref="InstallFolderVariable"/> is the engine's and must not redirect an install).
        /// </summary>
        /// <exception cref="LaunchArgumentException">A folder is named nowhere, or is not a valid path.</exception>
        public static InstallLocations ForHeadlessInstall(LaunchArguments launch, Func<string, string?> environment)
        {
            string game = launch.GameDirectory ?? Named(environment, GameFolderVariable)
                ?? throw new LaunchArgumentException($"The Fallout 4 folder is unknown: pass {LaunchArguments.GameDirectoryFlag} <folder> or set {GameFolderVariable}.");
            string install = launch.InstallDirectory
                ?? throw new LaunchArgumentException($"The folder to install into is unknown: pass {LaunchArguments.InstallDirectoryFlag} <folder>.");
            return new InstallLocations(Full(game), Full(install), launch.SettingsDirectory is { } settings ? Full(settings) : null);
        }

        internal static string? Named(Func<string, string?> environment, string variable) =>
            environment(variable) is { } value && !string.IsNullOrWhiteSpace(value) ? value : null;

        internal static string Full(string folder)
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
