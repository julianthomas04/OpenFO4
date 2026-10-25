using AssetConverter.Pipeline;

namespace AssetConverter
{
    /// <summary>
    /// The collision sidecars (<c>*.col.json</c>) below the mesh output and the stamp that says which extractor wrote them. The worker keeps
    /// existing sidecars (<c>--skip-existing</c>), so without the stamp a changed extractor would silently leave the old output in place.
    /// </summary>
    internal static class CollisionSidecars
    {
        /// <summary>Raised whenever the collision worker (or the staged input it reads) changes what it writes for the same game files.</summary>
        internal const int ExtractorVersion = 2;

        private const string SidecarPattern = "*.col.json";
        private const string StampFileName = ".collision.stamp";

        public static ConversionStamp Stamp { get; } = new(ExtractorVersion, null, []);

        public static string StampPath(string meshesOut) => Path.Combine(meshesOut, StampFileName);

        /// <returns>Why the sidecars below <paramref name="meshesOut"/> are not the output of this extractor version, or <c>null</c> when they are.</returns>
        public static string? StaleReason(string meshesOut) => Stamp.StaleReasonAt(StampPath(meshesOut));

        /// <summary>Deletes every sidecar when they are stale, so the extraction that follows rewrites all of them; stamped current ones are kept.</summary>
        public static void PurgeIfStale(string meshesOut, CancellationToken ct = default)
        {
            string? reason = StaleReason(meshesOut);
            if (reason == null || !HasSidecars(meshesOut)) return;
            Console.WriteLine($"Existing collision sidecars are stale ({reason}); they are extracted again.");
            Purge(meshesOut, ct);
        }

        /// <summary>Deletes every sidecar and the stamp, the stamp first so an interrupted purge never leaves a stamp vouching for a partial set.</summary>
        public static void Purge(string meshesOut, CancellationToken ct = default)
        {
            File.Delete(StampPath(meshesOut));
            InstallOperations.PurgeByExtension(meshesOut, SidecarPattern, "collision sidecars", ct: ct);
        }

        /// <summary>Records that the sidecars below <paramref name="meshesOut"/> are complete output of this extractor version; only called after a successful extraction.</summary>
        public static void RecordCurrent(string meshesOut)
        {
            Directory.CreateDirectory(meshesOut);
            Stamp.WriteTo(StampPath(meshesOut));
        }

        private static bool HasSidecars(string meshesOut) =>
            Directory.Exists(meshesOut) && Directory.EnumerateFiles(meshesOut, SidecarPattern, SearchOption.AllDirectories).Any();
    }
}
