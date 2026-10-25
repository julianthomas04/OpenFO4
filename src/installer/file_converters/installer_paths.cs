namespace AssetConverter
{
    /// <summary>
    /// The resources of the running installer (python worker, ESM schema, vendored tools, the game it installs), looked up on first use:
    /// those of its own payload when it is a packaged installer, otherwise the repo's, resolved relative to the repo root. Nothing is
    /// copied to the build output.
    /// </summary>
    public static class InstallerPaths
    {
        const string RootMarker = "src/installer/installer.csproj";

        static readonly object _resourcesGate = new();
        static InstallerResources? _resources;

        /// <summary>The resources; the first call unpacks the payload of a packaged installer, which the installer's own first step does with progress through <see cref="Unpack"/>.</summary>
        internal static InstallerResources Resources => Unpack(null);

        /// <summary>The resources, reporting the unpacking of the payload (once, by the first caller) to <paramref name="progress"/>.</summary>
        /// <exception cref="InstallStartupException">The payload could not be unpacked.</exception>
        internal static InstallerResources Unpack(UnpackProgress? progress)
        {
            lock (_resourcesGate) return _resources ??= InstallerResourceLocator.Locate(typeof(InstallerPaths).Assembly, progress);
        }

        /// <summary>True for an installer that carries its own payload; known without unpacking it.</summary>
        internal static bool IsPackaged { get; } = InstallerResourceLocator.IsPackaged(typeof(InstallerPaths).Assembly);

        /// <summary>Absolute path of a resource given relative to the resource root; throws if it does not exist.</summary>
        public static string Require(string repoRelativePath) => Resources.Require(repoRelativePath);

        internal static string FindRepoRoot() => FindRepoRoot([AppContext.BaseDirectory, Environment.CurrentDirectory]);

        /// Walks up from each start directory (in order) to the first ancestor containing the root marker.
        internal static string FindRepoRoot(IReadOnlyList<string> startDirectories)
        {
            foreach (string start in startDirectories)
            {
                for (var dir = new DirectoryInfo(start); dir != null; dir = dir.Parent)
                    if (File.Exists(Path.Combine(dir.FullName, RootMarker)))
                        return dir.FullName;
            }
            throw new DirectoryNotFoundException(
                $"Repo root not found: no ancestor of {string.Join(" or ", startDirectories.Select(d => $"'{d}'"))} contains '{RootMarker}'.");
        }
    }
}
