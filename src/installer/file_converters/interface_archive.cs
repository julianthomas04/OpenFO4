using System.IO;
using SharpBSABA2;
using SharpBSABA2.BA2Util;

namespace AssetConverter
{
    /// Extracts individual files from `Fallout4 - Interface.ba2` (the Scaleform
    /// UI archive). Shared by the font, UI layout and Pip-Boy art extractors; both
    /// pull a single `.swf` out of this ~480 MB archive rather than unpacking it.
    public static class InterfaceArchive
    {
        /// Extract the one entry whose path ends with `relPathSuffix` (e.g.
        /// `Interface\fonts_en.swf`) into `stagingRoot`, preserving its archive
        /// folder, and return the full path written. Throws on any failure
        /// (missing archive, corrupt/locked archive, entry absent).
        public static string ExtractFile(string fo4Location, string relPathSuffix, string stagingRoot)
        {
            (BA2 ba2, string archivePath) = Open(fo4Location);

            string relPath;
            try
            {
                string suffix = relPathSuffix.Replace('/', '\\');
                ArchiveEntry? entry = ba2.Files.FirstOrDefault(f =>
                    f.FullPath.Replace('/', '\\').EndsWith(suffix, StringComparison.OrdinalIgnoreCase));
                if (entry == null)
                    throw new InterfaceArchiveException($"'{suffix}' not found in '{archivePath}'.");
                relPath = entry.FullPath.Replace('/', '\\');
                Directory.CreateDirectory(stagingRoot);
                entry.Extract(stagingRoot, true);
            }
            finally
            {
                ba2.Close();
            }

            string outPath = Path.Combine(stagingRoot, relPath);
            if (!File.Exists(outPath))
                throw new InterfaceArchiveException(
                    $"Expected '{outPath}' after BA2 extraction; not found.");
            return outPath;
        }

        /// Extract every entry whose path (backslashes, below the archive root) satisfies
        /// `include` into `stagingRoot`, opening the archive once. Returns the full paths written.
        public static IReadOnlyList<string> ExtractMatching(string fo4Location, Func<string, bool> include, string stagingRoot)
        {
            (BA2 ba2, _) = Open(fo4Location);

            var written = new List<string>();
            try
            {
                Directory.CreateDirectory(stagingRoot);
                foreach (ArchiveEntry entry in ba2.Files.Where(f => include(f.FullPath.Replace('/', '\\'))))
                {
                    entry.Extract(stagingRoot, true);
                    string outPath = Path.Combine(stagingRoot, entry.FullPath.Replace('/', '\\'));
                    if (!File.Exists(outPath))
                        throw new InterfaceArchiveException($"Expected '{outPath}' after BA2 extraction; not found.");
                    written.Add(outPath);
                }
            }
            finally
            {
                ba2.Close();
            }
            return written;
        }

        /// Read every entry whose path (backslashes, below the archive root) satisfies
        /// `include` into memory, opening the archive once. Returns (archive path, bytes)
        /// pairs in archive order.
        public static IReadOnlyList<(string Path, byte[] Content)> ReadMatching(string fo4Location, Func<string, bool> include)
        {
            (BA2 ba2, _) = Open(fo4Location);
            try
            {
                var read = new List<(string, byte[])>();
                foreach (ArchiveEntry entry in ba2.Files.Where(f => include(f.FullPath.Replace('/', '\\'))))
                {
                    using MemoryStream data = entry.GetDataStream();
                    read.Add((entry.FullPath.Replace('/', '\\'), data.ToArray()));
                }
                return read;
            }
            finally
            {
                ba2.Close();
            }
        }

        static (BA2 Archive, string ArchivePath) Open(string fo4Location)
        {
            string archivePath = Path.Combine(fo4Location, "Data", "Fallout4 - Interface.ba2");
            if (!File.Exists(archivePath))
                throw new InterfaceArchiveException($"'{archivePath}' not found — the Fallout 4 install is incomplete.");
            try
            {
                return (new BA2(archivePath), archivePath);
            }
            catch (Exception ex) when (ex is IOException or InvalidDataException or UnauthorizedAccessException)
            {
                throw new InterfaceArchiveException(
                    $"Failed to open '{archivePath}' (corrupt or locked).", ex);
            }
        }
    }
}
