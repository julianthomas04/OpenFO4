using System.Collections.Concurrent;

namespace AssetConverter.Pipeline
{
    /// <summary>
    /// Gives a staged file the spelling its path has on disk. The archives name the same folder in different cases ("Meshes\Actors" in
    /// one entry, "meshes\actors" in another) and the staging tree keeps whichever spelling created the folder first, so the on-disk
    /// spelling is the only one every file of a folder shares, whatever order the files are handled in. A file keeps the spelling it
    /// was first created with as well: extracting over a file an earlier run left in another case does not rename it.
    /// </summary>
    /// <remarks>
    /// Meant for the files this run writes. Each folder is looked up once, and the names it held when it was first asked about are
    /// remembered. A file missing from that snapshot was created after it, by this run, which writes every file once and so created it
    /// with the spelling it is asked about.
    /// </remarks>
    internal sealed class StagedPathSpelling(string root)
    {
        private static readonly EnumerationOptions IgnoreCase = new() { MatchCasing = MatchCasing.CaseInsensitive };

        private sealed class Folder(string fullPath)
        {
            private readonly Lazy<Dictionary<string, string>> _names = new(() => NamesIn(fullPath));

            public string FullPath => fullPath;

            public string? NameOnDisk(string name) => _names.Value.GetValueOrDefault(name);

            private static Dictionary<string, string> NamesIn(string folder) =>
                Directory.EnumerateFileSystemEntries(folder).Select(Path.GetFileName)
                    .ToDictionary(name => name!, name => name!, StringComparer.OrdinalIgnoreCase);
        }

        private readonly ConcurrentDictionary<string, Folder> _folders = new(StringComparer.OrdinalIgnoreCase);

        /// <returns>The full path of the staged file <paramref name="relativePath"/>, as it is spelled on disk.</returns>
        /// <exception cref="FileNotFoundException">No such file or folder is staged.</exception>
        public string Resolve(string relativePath)
        {
            string normalized = ArchivePath.Normalize(relativePath);
            int nameStart = normalized.LastIndexOf('\\') + 1;
            string name = normalized[nameStart..];
            Folder folder = FolderOf(normalized[..Math.Max(nameStart - 1, 0)]);

            string? onDisk = folder.NameOnDisk(name);
            string path = Path.Combine(folder.FullPath, onDisk ?? name);
            if (onDisk == null && !File.Exists(path) && !Directory.Exists(path))
                throw new FileNotFoundException($"'{name}' is not staged in '{folder.FullPath}'.", path);
            return path;
        }

        /// <param name="relativeFolder">Backslash separated, empty for the staging root.</param>
        private Folder FolderOf(string relativeFolder) =>
            _folders.GetOrAdd(relativeFolder, static (key, self) => self.OpenFolder(key), this);

        private Folder OpenFolder(string relativeFolder)
        {
            if (relativeFolder.Length == 0) return new Folder(root);
            int nameStart = relativeFolder.LastIndexOf('\\') + 1;
            string parent = FolderOf(relativeFolder[..Math.Max(nameStart - 1, 0)]).FullPath;
            string name = relativeFolder[nameStart..];
            string onDisk = Directory.EnumerateDirectories(parent, name, IgnoreCase).FirstOrDefault()
                ?? throw new FileNotFoundException($"'{name}' is not staged in '{parent}'.", Path.Combine(parent, name));
            return new Folder(onDisk);
        }
    }
}
