using AssetConverter.Pipeline;

namespace AssetConverter.Audio
{
    /// <summary>A source entry to convert: its archive path, the key of its output below the audio folder and how to read it.</summary>
    internal sealed record AudioEntry(string Path, string Key, AudioContainer Container);

    /// <summary>What each archive converts: its in-scope entries, minus those a later archive replaces, checked for outputs that would overwrite each other.</summary>
    internal static class AudioPlan
    {
        /// <param name="entryPathsPerArchive">Every entry path of each archive, in the order the archives are applied.</param>
        /// <returns>For each archive, the entries it converts.</returns>
        /// <exception cref="AudioConversionException">An in-scope entry is not a known audio format, or two different entries convert to the same output.</exception>
        public static IReadOnlyList<IReadOnlyList<AudioEntry>> Build(IReadOnlyList<IEnumerable<string>> entryPathsPerArchive)
        {
            var inScope = entryPathsPerArchive.Select(paths => paths.Where(AudioLayout.IsInScope).ToArray()).ToArray();
            var overlap = new ArchiveOverlap(inScope, path => path);
            IReadOnlyList<AudioEntry>[] archives =
            [
                .. inScope.Select((paths, archive) => (IReadOnlyList<AudioEntry>)
                    [.. LastPerPath(paths.Where(path => overlap.IsLastHolder(archive, path))).Select(ToEntry)]),
            ];
            RejectSharedOutputs(archives.SelectMany(entries => entries));
            return archives;
        }

        private static Dictionary<string, string>.ValueCollection LastPerPath(IEnumerable<string> paths)
        {
            var last = new Dictionary<string, string>(ArchivePath.Comparer);
            foreach (string path in paths) last[path] = path;
            return last.Values;
        }

        private static AudioEntry ToEntry(string path) => new(path, AudioLayout.ToAudioKey(path), AudioLayout.ContainerOf(path));

        private static void RejectSharedOutputs(IEnumerable<AudioEntry> entries)
        {
            var shared = entries.GroupBy(entry => entry.Key, StringComparer.Ordinal).Where(group => group.Count() > 1).ToList();
            if (shared.Count == 0) return;
            string description = string.Join("; ", shared.Take(10).Select(group =>
                $"{string.Join(" and ", group.Select(entry => $"'{entry.Path}'"))} both convert to '{InstallLayout.AudioFolder}/{group.Key}'"));
            throw new AudioConversionException(
                $"{shared.Count:N0} output file(s) would be written by more than one audio entry: {description}{(shared.Count > 10 ? "; ..." : "")}.");
        }
    }
}
