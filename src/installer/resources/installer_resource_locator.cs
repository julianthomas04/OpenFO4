using System.Reflection;

namespace AssetConverter
{
    /// <summary>Decides which resources the running installer uses: its own payload when it carries one, otherwise the repository checkout it runs from.</summary>
    internal static class InstallerResourceLocator
    {
        public const string PayloadArchiveResource = "OpenFO4.Payload.zip";
        public const string PayloadIdResource = "OpenFO4.Payload.id";

        public static string DefaultCacheRoot { get; } = Path.Combine(
            Environment.GetFolderPath(Environment.SpecialFolder.LocalApplicationData), "OpenFO4", "installer");

        private static readonly List<PayloadFolder> HeldFolders = [];

        public static bool IsPackaged(Assembly assembly) => assembly.GetManifestResourceNames().Contains(PayloadArchiveResource);

        /// <summary>
        /// The resources of the installer in <paramref name="assembly"/>. A packaged installer unpacks its payload, which the process then holds
        /// until it ends, and removes the folders of other versions from the cache.
        /// </summary>
        /// <exception cref="InstallStartupException">The payload could not be unpacked.</exception>
        public static InstallerResources Locate(Assembly assembly, UnpackProgress? progress = null)
        {
            var payload = EmbeddedPayload(assembly);
            if (payload == null) return InstallerResources.ForRepository(InstallerPaths.FindRepoRoot());
            var folder = PayloadExtractor.Unpack(payload, DefaultCacheRoot, InstallerVersion.Read(assembly), progress, Console.Error.WriteLine);
            lock (HeldFolders) HeldFolders.Add(folder);
            return InstallerResources.ForPayload(folder.Path);
        }

        /// <param name="payload">The payload the installer carries; null for an installer run from the repository.</param>
        /// <param name="findRepoRoot">Finds the repository checkout; only asked for when there is no payload.</param>
        /// <exception cref="InstallStartupException">The payload could not be unpacked.</exception>
        internal static InstallerResources Locate(PayloadSource? payload, string cacheRoot, string version, Func<string> findRepoRoot) =>
            payload == null
                ? InstallerResources.ForRepository(findRepoRoot())
                : InstallerResources.ForPayload(PayloadExtractor.EnsureExtracted(payload, cacheRoot, version));

        private static PayloadSource? EmbeddedPayload(Assembly assembly)
        {
            if (!IsPackaged(assembly)) return null;
            using var idStream = assembly.GetManifestResourceStream(PayloadIdResource)
                ?? throw new InvalidOperationException($"The installer carries a payload but not its identifier ('{PayloadIdResource}'); the build is broken.");
            using var reader = new StreamReader(idStream);
            return new PayloadSource(
                () => assembly.GetManifestResourceStream(PayloadArchiveResource)!,
                reader.ReadToEnd().Trim());
        }
    }
}
