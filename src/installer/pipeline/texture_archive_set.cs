using SharpBSABA2.BA2Util;

namespace AssetConverter.Pipeline
{
    /// <summary>
    /// The texture archives that still need extracting into the install: those without a done-marker that exist in the game's data
    /// folder, opened up front so their name tables can seed the texture index before any texture byte is read. Disposing closes every
    /// archive that is still open, so a run that stops before the extraction does not keep the files locked.
    /// </summary>
    internal sealed class TextureArchiveSet : IDisposable
    {
        public readonly record struct Entry(string Name, BA2 Archive);

        private readonly List<Entry> _pending;
        private readonly string _stagingRoot;

        private TextureArchiveSet(List<Entry> pending, string stagingRoot)
        {
            _pending = pending;
            _stagingRoot = stagingRoot;
        }

        public IReadOnlyList<Entry> Pending => _pending;

        public static TextureArchiveSet Open(string gameDataDirectory, string stagingRoot, IEnumerable<string> archiveNames)
        {
            var pending = new List<Entry>();
            try
            {
                foreach (var name in archiveNames)
                {
                    string archivePath = Path.Combine(gameDataDirectory, name);
                    if (File.Exists(MarkerPath(stagingRoot, name))) continue;
                    if (!File.Exists(archivePath))
                    {
                        Console.WriteLine($"WARNING: {archivePath} not found. Skipping.");
                        continue;
                    }
                    pending.Add(new Entry(name, new BA2(archivePath)));
                }
            }
            catch
            {
                foreach (var entry in pending) entry.Archive.Close();
                throw;
            }
            return new TextureArchiveSet(pending, stagingRoot);
        }

        /// <summary>
        /// Every texture the install will have: the pending archives' entries, then (warm re-runs / mixed states) the already-deployed
        /// files. The order feeds the converter's by-filename lookup.
        /// </summary>
        public IEnumerable<string> IndexNames(string texturesOut)
        {
            foreach (var entry in _pending)
                foreach (var file in entry.Archive.Files)
                    yield return file.FullPath;
            if (Directory.Exists(texturesOut))
                foreach (var file in Directory.EnumerateFiles(texturesOut, "*.dds", SearchOption.AllDirectories))
                    yield return Path.GetRelativePath(texturesOut, file);
        }

        /// <summary>Records that every entry of the archive was extracted, so the next run skips it.</summary>
        public void MarkExtracted(Entry entry) => File.WriteAllText(MarkerPath(_stagingRoot, entry.Name), "");

        public void Dispose()
        {
            foreach (var entry in _pending) entry.Archive.Close();
        }

        private static string MarkerPath(string stagingRoot, string archiveName) => Path.Combine(stagingRoot, $".{archiveName}.extracted-direct");
    }
}
