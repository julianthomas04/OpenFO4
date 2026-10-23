using System.Reflection;

namespace AssetConverter
{
    /// <summary>The version this installer was built as: <c>-p:Version=</c> of the release build (whatever <c>build-release.ps1 -Version</c> received), or the csproj default for a dev build.</summary>
    internal static class InstallerVersion
    {
        /// <summary>The characters a version may hold; the same rule as <c>Assert-ValidVersion</c> in build-release.ps1, which keeps MSBuild, file names and the marker free of surprises.</summary>
        public const string Pattern = "[0-9A-Za-z][0-9A-Za-z._-]*";

        public static string Current { get; } = Read(typeof(InstallerVersion).Assembly);

        internal static string Read(Assembly assembly)
        {
            string? informational = assembly.GetCustomAttribute<AssemblyInformationalVersionAttribute>()?.InformationalVersion;
            return informational ?? assembly.GetName().Version?.ToString() ?? "unknown";
        }
    }
}
