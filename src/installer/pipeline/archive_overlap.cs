namespace AssetConverter.Pipeline
{
    /// <summary>
    /// Which archive finally holds each destination file when several archives are extracted one after another into the same folder:
    /// the last archive that contains it. Extracting only the last holder's entry writes every file exactly once and leaves what the
    /// sequential extraction would leave, so archives can be extracted at the same time without two streams writing one file.
    /// Destinations compare ignoring case and slash direction, like the file system the installer writes to; an archive listing a destination twice writes it once.
    /// </summary>
    internal sealed class ArchiveOverlap
    {
        private readonly Dictionary<string, int> _lastHolder = new(ArchivePath.Comparer);
        private readonly Func<string, string> _destinationOf;
        private readonly int[] _fileCounts;

        /// <param name="entryPathsPerArchive">The entries each archive contributes, in extraction order.</param>
        /// <param name="destinationOf">Where an entry path lands below the extraction folder.</param>
        public ArchiveOverlap(IReadOnlyList<IEnumerable<string>> entryPathsPerArchive, Func<string, string> destinationOf)
        {
            _destinationOf = destinationOf;
            var entries = entryPathsPerArchive.Select(paths => paths.ToArray()).ToArray();
            for (int archive = 0; archive < entries.Length; archive++)
                foreach (string path in entries[archive])
                    _lastHolder[destinationOf(path)] = archive;
            _fileCounts = entries.Select((paths, archive) => paths
                .Where(path => IsLastHolder(archive, path)).Select(destinationOf).Distinct(ArchivePath.Comparer).Count()).ToArray();
            TotalFileCount = _fileCounts.Sum();
        }

        /// <summary>The files all archives write together: every entry except those a later archive overwrites.</summary>
        public int TotalFileCount { get; }

        public int FileCount(int archiveIndex) => _fileCounts[archiveIndex];

        /// <summary>Whether the archive at <paramref name="archiveIndex"/> is the one that writes the destination of <paramref name="entryPath"/>.</summary>
        public bool IsLastHolder(int archiveIndex, string entryPath) =>
            _lastHolder.TryGetValue(_destinationOf(entryPath), out int holder) && holder == archiveIndex;
    }
}
