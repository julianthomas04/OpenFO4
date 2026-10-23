namespace AssetConverter
{
    /// <summary>A command-line argument that cannot be used; the message says why and is meant to be shown as it is.</summary>
    internal sealed class LaunchArgumentException(string message) : Exception(message);

    /// <summary>
    /// The switches of a full install, from the command line and the environment (either one turns a switch on), and the folders a
    /// headless install names (the settings folder holds the INIs the game writes on its first launch; the variable only applies when no flag is given). The folder flags take their value as the next argument or after an equals sign.
    /// </summary>
    internal sealed record LaunchArguments(bool Headless, bool KeepStaging, string? InstallDirectory = null, string? GameDirectory = null, bool RemoveStaging = false, string? SettingsDirectory = null)
    {
        public const string HeadlessFlag = "--headless";
        public const string HeadlessVariable = "OPENFO4_HEADLESS";
        public const string KeepStagingFlag = "--keep-staging";
        public const string KeepStagingVariable = "OPENFO4_KEEP_STAGING";
        public const string RemoveStagingFlag = "--remove-staging";
        public const string RemoveStagingVariable = "OPENFO4_REMOVE_STAGING";
        public const string InstallDirectoryFlag = "--install-dir";
        public const string GameDirectoryFlag = "--game-dir";
        public const string SettingsDirectoryFlag = "--settings-dir";
        public const string SettingsDirectoryVariable = "OPENFO4_SETTINGS_DIR";

        /// <summary>Whether the temporary files stay after a successful install: a window install asks the user (<see cref="KeepStaging"/> only pre-ticks the checkbox), a headless install keeps them unless told to remove them.</summary>
        public bool KeepsStagingWhenHeadless => !RemoveStaging;

        /// <exception cref="LaunchArgumentException">A folder flag has no value, or both <see cref="KeepStagingFlag"/> and <see cref="RemoveStagingFlag"/> are given.</exception>
        public static LaunchArguments Parse(IReadOnlyList<string> args, Func<string, string?> getEnvironmentVariable)
        {
            var parsed = new LaunchArguments(
                IsOn(args, HeadlessFlag, HeadlessVariable, getEnvironmentVariable),
                IsOn(args, KeepStagingFlag, KeepStagingVariable, getEnvironmentVariable),
                ValueOf(args, InstallDirectoryFlag),
                ValueOf(args, GameDirectoryFlag),
                IsOn(args, RemoveStagingFlag, RemoveStagingVariable, getEnvironmentVariable),
                ValueOf(args, SettingsDirectoryFlag) ?? getEnvironmentVariable(SettingsDirectoryVariable));
            if (parsed.KeepStaging && parsed.RemoveStaging)
                throw new LaunchArgumentException($"{KeepStagingFlag} and {RemoveStagingFlag} (or {KeepStagingVariable} and {RemoveStagingVariable}) contradict each other; use only one.");
            return parsed;
        }

        private static bool IsOn(IReadOnlyList<string> args, string flag, string variable, Func<string, string?> getEnvironmentVariable) =>
            args.Contains(flag) || getEnvironmentVariable(variable) == "1";

        private static string? ValueOf(IReadOnlyList<string> args, string flag)
        {
            for (int i = 0; i < args.Count; i++)
            {
                if (args[i].StartsWith(flag + "=", StringComparison.Ordinal)) return RequireValue(flag, args[i][(flag.Length + 1)..]);
                if (args[i] == flag) return RequireValue(flag, i + 1 < args.Count ? args[i + 1] : "");
            }
            return null;
        }

        private static string RequireValue(string flag, string value) =>
            value.Trim().Length > 0 && !value.StartsWith("--", StringComparison.Ordinal)
                ? value
                : throw new LaunchArgumentException($"{flag} needs a folder, for example {flag} \"D:\\Games\\OpenFO4\".");
    }
}
