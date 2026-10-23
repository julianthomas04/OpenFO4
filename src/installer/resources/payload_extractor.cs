using System.IO.Compression;
using System.Text.RegularExpressions;

namespace AssetConverter
{
    /// <summary>The payload archive a packaged installer carries and the content hash that identifies it.</summary>
    internal sealed record PayloadSource(Func<Stream> Open, string Id);

    /// <summary>Reports the unpacking of the payload: the number of files it holds, known once the archive is open, and each file as it is written.</summary>
    internal sealed record UnpackProgress(Action<long> FileCount, Action FileUnpacked);

    /// <summary>
    /// Unpacks the payload into a folder of its own per installer version and payload content, once: a folder that holds the completion
    /// marker is reused, and anything else (an earlier run that was interrupted) is replaced. The unpacking happens in a sibling folder
    /// that is renamed into place after the marker is written, so a folder under the final name is either complete or absent. Concurrent
    /// installers wait for each other (<see cref="PayloadFolderLock"/>).
    /// </summary>
    internal static partial class PayloadExtractor
    {
        public const string CompletionMarker = ".payload-complete";

        private const int KeyIdLength = 12;
        private static readonly TimeSpan LockTimeout = TimeSpan.FromMinutes(10);

        /// <summary>The folder holding the extracted payload, which nothing keeps from being deleted as stale.</summary>
        /// <exception cref="InstallStartupException">The payload could not be unpacked.</exception>
        public static string EnsureExtracted(PayloadSource payload, string cacheRoot, string version, UnpackProgress? progress = null) =>
            Ensure(payload, cacheRoot, version, progress, hold: false, out _);

        /// <summary>
        /// Unpacks the payload like <see cref="EnsureExtracted"/> and returns the folder as one the running installer holds, then removes the
        /// payload folders of other versions and the leftovers of interrupted unpackings that no other installer holds. Dispose the result
        /// when the installer no longer reads from the folder.
        /// </summary>
        /// <param name="warn">Receives a message for each old folder that could not be removed; removing them never fails the unpacking.</param>
        /// <exception cref="InstallStartupException">The payload could not be unpacked.</exception>
        public static PayloadFolder Unpack(PayloadSource payload, string cacheRoot, string version, UnpackProgress? progress, Action<string> warn)
        {
            string target = Ensure(payload, cacheRoot, version, progress, hold: true, out var folder);
            StalePayloadFolders.Remove(cacheRoot, target, warn);
            return folder!;
        }

        internal static string FolderName(string version, string payloadId)
        {
            if (!PayloadIdPattern().IsMatch(payloadId)) throw new ArgumentException($"The payload id '{payloadId}' is not a hex content hash.", nameof(payloadId));
            string safeVersion = string.Concat(version.Select(c => Path.GetInvalidFileNameChars().Contains(c) ? '_' : c));
            return $"{safeVersion}-{payloadId[..Math.Min(KeyIdLength, payloadId.Length)].ToLowerInvariant()}";
        }

        private static string Ensure(PayloadSource payload, string cacheRoot, string version, UnpackProgress? progress, bool hold, out PayloadFolder? folder)
        {
            string target = Path.Combine(cacheRoot, FolderName(version, payload.Id));
            folder = null;
            try
            {
                using var gate = PayloadFolderLock.Acquire(target, LockTimeout);
                if (!IsComplete(target, payload.Id)) Extract(payload, cacheRoot, target, progress);
                if (hold) folder = PayloadFolder.Hold(target);
                return target;
            }
            catch (Exception ex) when (ex is IOException or UnauthorizedAccessException or InvalidDataException or TimeoutException)
            {
                throw new InstallStartupException($"The installer could not unpack its files into '{target}': {ex.Message}", ex);
            }
        }

        private static bool IsComplete(string folder, string payloadId)
        {
            string marker = Path.Combine(folder, CompletionMarker);
            return File.Exists(marker) && File.ReadAllText(marker).Trim().Equals(payloadId, StringComparison.OrdinalIgnoreCase);
        }

        private static void Extract(PayloadSource payload, string cacheRoot, string target, UnpackProgress? progress)
        {
            string unpacking = target + ".unpacking";
            DeleteFolder(unpacking);
            DeleteFolder(target);
            Directory.CreateDirectory(cacheRoot);
            using (var stream = payload.Open())
            using (var archive = new ZipArchive(stream, ZipArchiveMode.Read))
                ExtractEntries(archive, unpacking, progress);
            File.WriteAllText(Path.Combine(unpacking, CompletionMarker), payload.Id);
            Directory.Move(unpacking, target);
        }

        private static void ExtractEntries(ZipArchive archive, string folder, UnpackProgress? progress)
        {
            string root = Path.GetFullPath(folder) + Path.DirectorySeparatorChar;
            Directory.CreateDirectory(folder);
            progress?.FileCount(archive.Entries.Count(entry => !IsDirectory(entry)));
            foreach (var entry in archive.Entries)
            {
                string destination = Path.GetFullPath(Path.Combine(folder, entry.FullName));
                if (!destination.StartsWith(root, StringComparison.OrdinalIgnoreCase))
                    throw new InvalidDataException($"The payload entry '{entry.FullName}' lies outside the folder it is unpacked into.");
                if (IsDirectory(entry))
                {
                    Directory.CreateDirectory(destination);
                    continue;
                }
                Directory.CreateDirectory(Path.GetDirectoryName(destination)!);
                entry.ExtractToFile(destination, overwrite: true);
                progress?.FileUnpacked();
            }
        }

        private static bool IsDirectory(ZipArchiveEntry entry) => entry.FullName.EndsWith('/') || entry.FullName.EndsWith('\\');

        private static void DeleteFolder(string folder)
        {
            if (Directory.Exists(folder)) Directory.Delete(folder, recursive: true);
        }

        [GeneratedRegex("^[0-9a-fA-F]{8,}$")]
        private static partial Regex PayloadIdPattern();
    }
}
