using AssetConverter.Pipeline;

namespace AssetConverter
{
    /// <summary>The folders a dev subcommand works on and the arguments that remain once the folder options are taken out of them.</summary>
    internal sealed record DevCommandTarget(InstallLocations Locations, string[] Arguments);

    /// <summary>
    /// Resolves <c>--install-dir</c> and <c>--game-dir</c> for the dev subcommands, parsed by <see cref="LaunchArguments"/> like a headless install's.
    /// An option that is given is validated; one that is omitted keeps the development default, as it is.
    /// </summary>
    internal static class DevCommandLocations
    {
        /// <param name="args">The full argument list; <c>args[0]</c> is the subcommand.</param>
        /// <param name="defaultGameFolder">The game folder when <c>--game-dir</c> is omitted.</param>
        /// <param name="defaultInstallFolder">The install root when <c>--install-dir</c> is omitted.</param>
        /// <exception cref="LaunchArgumentException">An option has no value or names a folder that cannot be used.</exception>
        public static DevCommandTarget Resolve(string[] args, Func<string, string?> getEnvironmentVariable, string defaultGameFolder, string? defaultInstallFolder)
        {
            var launch = LaunchArguments.Parse(args, getEnvironmentVariable);
            string game = launch.GameDirectory is { } chosenGame ? ValidGameFolder(chosenGame) : defaultGameFolder;
            string install = launch.InstallDirectory is { } chosenInstall
                ? ValidInstallRoot(chosenInstall)
                : defaultInstallFolder ?? throw new LaunchArgumentException($"Pass {LaunchArguments.InstallDirectoryFlag} <folder>: there is no default install folder.");
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
            string full = Full(chosen);
            if (!Directory.Exists(Path.Combine(full, "Data")))
                throw new LaunchArgumentException($"{LaunchArguments.GameDirectoryFlag} '{full}' is not a Fallout 4 folder: it has no Data folder.");
            return full;
        }

        private static string ValidInstallRoot(string chosen)
        {
            string root = InstallFolderRules.InstallRoot(Full(chosen));
            if (!new InstallLocations("", root).HoldsInstall())
                throw new LaunchArgumentException($"{LaunchArguments.InstallDirectoryFlag} '{chosen}' does not hold an OpenFO4 install (looked in {root}).");
            return root;
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
