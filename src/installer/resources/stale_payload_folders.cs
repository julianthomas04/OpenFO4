using System.Text.RegularExpressions;

namespace AssetConverter
{
    /// <summary>
    /// Deletes the payload folders of other installer versions and the leftovers of interrupted unpackings from the payload cache, so the
    /// cache does not grow by a payload per release. Best effort: a folder is skipped while another installer holds it (its lock, or
    /// its marker, see <see cref="PayloadFolder"/>), and what cannot be deleted is reported as a warning and left for the next run.
    /// </summary>
    internal static partial class StalePayloadFolders
    {
        private const string UnpackingSuffix = ".unpacking";

        /// <param name="current">The payload folder the running installer uses; never touched, nor is its unpacking sibling.</param>
        /// <param name="warn">Receives one message per folder that could not be removed.</param>
        public static void Remove(string cacheRoot, string current, Action<string> warn)
        {
            IEnumerable<string> folders;
            try
            {
                folders = Directory.Exists(cacheRoot) ? Directory.EnumerateDirectories(cacheRoot).ToList() : [];
            }
            catch (Exception ex) when (ex is IOException or UnauthorizedAccessException)
            {
                warn($"WARNING: could not look for old installer files in '{cacheRoot}': {ex.Message}");
                return;
            }
            foreach (string folder in folders.Where(folder => IsStale(folder, current)))
                TryRemove(folder, warn);
        }

        private static bool IsStale(string folder, string current)
        {
            string target = TargetOf(folder);
            return !target.Equals(current, StringComparison.OrdinalIgnoreCase) && PayloadFolderName().IsMatch(Path.GetFileName(target));
        }

        private static string TargetOf(string folder) =>
            folder.EndsWith(UnpackingSuffix, StringComparison.OrdinalIgnoreCase) ? folder[..^UnpackingSuffix.Length] : folder;

        private static void TryRemove(string folder, Action<string> warn)
        {
            try
            {
                using var gate = PayloadFolderLock.TryAcquire(TargetOf(folder));
                if (gate == null) return;
                if (!folder.EndsWith(UnpackingSuffix, StringComparison.OrdinalIgnoreCase) && PayloadFolder.IsHeld(folder)) return;
                Directory.Delete(folder, recursive: true);
            }
            catch (Exception ex) when (ex is IOException or UnauthorizedAccessException)
            {
                warn($"WARNING: could not remove the old installer files '{folder}': {ex.Message}");
            }
        }

        [GeneratedRegex("^.+-[0-9a-f]{8,}$")]
        private static partial Regex PayloadFolderName();
    }
}
