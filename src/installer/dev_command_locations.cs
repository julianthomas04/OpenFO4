using AssetConverter.Pipeline;

namespace AssetConverter
{
    /// <summary>The folders a dev subcommand works on and the arguments that remain once the folder options are taken out of them.</summary>
    internal sealed record DevCommandTarget(InstallLocations Locations, string[] Arguments);

    /// <summary>
    /// Resolves the folders of a dev subcommand, with <c>--install-dir</c> and <c>--game-dir</c> parsed by <see cref="LaunchArguments"/> like a headless install's:
    /// the game folder is the flag, else <see cref="HeadlessLocations.GameFolderVariable"/>, else what Steam knows; the install folder is the flag, else
    /// <see cref="HeadlessLocations.InstallFolderVariable"/>. A folder named by a flag or variable is validated.
    /// </summary>
    internal static class DevCommandLocations
    {
        /// <param name="args">The full argument list; <c>args[0]</c> is the subcommand.</param>
        /// <param name="detectGameFolder">The game folder Steam knows; asked only when neither the flag nor the variable names one.</param>
        /// <exception cref="LaunchArgumentException">An option has no value, a folder is named nowhere, or a named folder cannot be used.</exception>
        public static DevCommandTarget Resolve(string[] args, Func<string, string?> getEnvironmentVariable, Func<string?> detectGameFolder)
        {
            var launch = LaunchArguments.Parse(args, getEnvironmentVariable);
            string game = (launch.GameDirectory ?? HeadlessLocations.Named(getEnvironmentVariable, HeadlessLocations.GameFolderVariable)) is { } chosenGame
                ? ValidGameFolder(chosenGame)
                : detectGameFolder() ?? throw new LaunchArgumentException(
                    $"The Fallout 4 folder is unknown and Steam did not know it either: pass {LaunchArguments.GameDirectoryFlag} <folder> or set {HeadlessLocations.GameFolderVariable}.");
            string install = (launch.InstallDirectory ?? HeadlessLocations.Named(getEnvironmentVariable, HeadlessLocations.InstallFolderVariable)) is { } chosenInstall
                ? ValidInstallRoot(chosenInstall)
                : throw new LaunchArgumentException(
                    $"The install folder is unknown: pass {LaunchArguments.InstallDirectoryFlag} <folder> or set {HeadlessLocations.InstallFolderVariable}.");
            return new DevCommandTarget(new InstallLocations(game, install), WithoutFolderOptions(args));
        }

        /// <summary>The arguments without <c>--install-dir</c> and <c>--game-dir</c> and their values, so a subcommand parses only its own.</summary>
        public static string[] WithoutFolderOptions(IReadOnlyList<string> args)
        {
            var remaining = new List<string>();
            for (int i = 0; i < args.Count; i++)
            {
                string arg = args[i];
                if (IsFolderOptionWithInlineValue(arg)) continue;
                if (arg is LaunchArguments.InstallDirectoryFlag or LaunchArguments.GameDirectoryFlag)
                {
                    i++;
                    continue;
                }
                remaining.Add(arg);
            }
            return [.. remaining];
        }

        private static bool IsFolderOptionWithInlineValue(string arg) =>
            arg.StartsWith(LaunchArguments.InstallDirectoryFlag + "=", StringComparison.Ordinal)
            || arg.StartsWith(LaunchArguments.GameDirectoryFlag + "=", StringComparison.Ordinal);

        private static string ValidGameFolder(string chosen)
        {
            string full = HeadlessLocations.Full(chosen);
            if (!Directory.Exists(Path.Combine(full, "Data")))
                throw new LaunchArgumentException($"{LaunchArguments.GameDirectoryFlag} '{full}' is not a Fallout 4 folder: it has no Data folder.");
            return full;
        }

        private static string ValidInstallRoot(string chosen)
        {
            string root = InstallFolderRules.InstallRoot(HeadlessLocations.Full(chosen));
            if (!new InstallLocations("", root).HoldsInstall())
                throw new LaunchArgumentException($"{LaunchArguments.InstallDirectoryFlag} '{chosen}' does not hold an OpenFO4 install (looked in {root}).");
            return root;
        }
    }
}
