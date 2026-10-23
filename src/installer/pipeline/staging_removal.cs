using System.Collections.Concurrent;
using AssetConverter.Progress;

namespace AssetConverter.Pipeline
{
    /// <param name="Failures">One line per file or folder that could not be removed, naming the path and the reason.</param>
    internal sealed record StagingRemovalResult(int FilesRemoved, int FoldersRemoved, IReadOnlyList<string> Failures);

    /// <summary>
    /// Deletes the installer's staging folder, and only that: it must lie inside an OpenFO4 install folder, and links (junctions, symbolic
    /// links) inside it are deleted themselves, never followed. The extraction markers go first, so a run cut short (cancelled, or a file
    /// that stays busy) never leaves markers vouching for files that are gone: if a marker cannot be removed, nothing else is removed and
    /// the next install extracts again where markers are missing. Files that a scanner holds for a moment are retried; what still cannot
    /// be removed is reported in the result and the rest is removed regardless.
    /// </summary>
    internal static class StagingRemoval
    {
        private const string MarkerFailureNote = "the other temporary files were left in place so that no extraction marker outlives its files";

        public static StagingRemovalResult Run(string stagingRoot, int progressPhase = -1, IReadOnlyList<TimeSpan>? retryDelays = null, Action<TimeSpan>? wait = null, CancellationToken ct = default)
        {
            RequireStagingFolderOfAnInstall(stagingRoot);
            var root = new DirectoryInfo(stagingRoot);
            if (!root.Exists) return new StagingRemovalResult(0, 0, []);

            retryDelays ??= IoRetry.DefaultDelays;
            var failures = new ConcurrentBag<string>();
            var tree = StagingTree.Walk(root, failures);
            InstallProgress.AddTotal(progressPhase, tree.Files.Count + tree.Folders.Count + 1);

            int filesRemoved = 0;
            int foldersRemoved = 0;

            bool RemoveFile(FileInfo file, string failureNote = "")
            {
                bool removed = TryRemove(file.FullName, () => DeleteFile(file), failures, retryDelays, wait, failureNote, ct);
                if (removed) Interlocked.Increment(ref filesRemoved);
                InstallProgress.Tick(progressPhase);
                return removed;
            }

            var markers = tree.Files.Where(file => IsExtractionMarker(file, root)).ToList();
            foreach (var marker in markers)
            {
                if (!RemoveFile(marker, MarkerFailureNote)) return new StagingRemovalResult(filesRemoved, foldersRemoved, [.. failures.Order(StringComparer.OrdinalIgnoreCase)]);
            }

            var options = new ParallelOptions { MaxDegreeOfParallelism = Math.Min(Environment.ProcessorCount, IoParallelism.MaxThreads), CancellationToken = ct };
            Parallel.ForEach(tree.Files.Except(markers), options, file => RemoveFile(file));

            foreach (var folder in tree.Folders.OrderByDescending(folder => folder.FullName.Length).Append(root))
            {
                ct.ThrowIfCancellationRequested();
                if (TryRemove(folder.FullName, () => Directory.Delete(folder.FullName, recursive: false), failures, retryDelays, wait, ct: ct)) foldersRemoved++;
                InstallProgress.Tick(progressPhase);
            }

            return new StagingRemovalResult(filesRemoved, foldersRemoved, [.. failures.Order(StringComparer.OrdinalIgnoreCase)]);
        }

        private static readonly EnumerationOptions AllEntries = new() { AttributesToSkip = 0, IgnoreInaccessible = false };

        private static bool IsExtractionMarker(FileInfo file, DirectoryInfo root) =>
            file.Name.StartsWith('.') && string.Equals(file.DirectoryName, root.FullName, StringComparison.OrdinalIgnoreCase);

        private static void DeleteFile(FileInfo file)
        {
            if (file.IsReadOnly) file.IsReadOnly = false;
            file.Delete();
        }

        private static bool TryRemove(string path, Action remove, ConcurrentBag<string> failures, IReadOnlyList<TimeSpan> retryDelays, Action<TimeSpan>? wait, string failureNote = "", CancellationToken ct = default)
        {
            try
            {
                IoRetry.Run(remove, IoRetry.IsFileBusyOrDenied, retryDelays, wait, ct);
                return true;
            }
            catch (Exception ex) when (ex is IOException or UnauthorizedAccessException)
            {
                failures.Add(failureNote.Length > 0 ? $"{path}: {ex.Message} ({failureNote})" : $"{path}: {ex.Message}");
                return false;
            }
        }

        private static void RequireStagingFolderOfAnInstall(string stagingRoot)
        {
            if (!Path.IsPathFullyQualified(stagingRoot) || Path.GetFileName(Path.TrimEndingDirectorySeparator(stagingRoot)) != InstallLayout.StagingFolder)
                throw new ArgumentException($"'{stagingRoot}' is not an installer staging folder; refusing to delete it.", nameof(stagingRoot));
            string? installRoot = Path.GetDirectoryName(Path.TrimEndingDirectorySeparator(stagingRoot));
            if (installRoot == null || !new InstallLocations("", installRoot).HoldsInstall())
                throw new ArgumentException($"'{stagingRoot}' is not inside an OpenFO4 install folder; refusing to delete it.", nameof(stagingRoot));
        }

        /// <summary>The files and folders below a staging folder. A link to a folder is listed as a folder but never entered.</summary>
        private sealed record StagingTree(IReadOnlyList<FileInfo> Files, IReadOnlyList<DirectoryInfo> Folders)
        {
            public static StagingTree Walk(DirectoryInfo root, ConcurrentBag<string> failures)
            {
                var files = new List<FileInfo>();
                var folders = new List<DirectoryInfo>();
                var pending = new Stack<DirectoryInfo>();
                if (!IsLink(root)) pending.Push(root);
                while (pending.Count > 0)
                {
                    var folder = pending.Pop();
                    try
                    {
                        foreach (var entry in folder.EnumerateFileSystemInfos("*", AllEntries))
                        {
                            if (entry is FileInfo file)
                            {
                                files.Add(file);
                                continue;
                            }
                            var child = (DirectoryInfo)entry;
                            folders.Add(child);
                            if (!IsLink(child)) pending.Push(child);
                        }
                    }
                    catch (Exception ex) when (ex is IOException or UnauthorizedAccessException)
                    {
                        failures.Add($"{folder.FullName}: {ex.Message}");
                    }
                }
                return new StagingTree(files, folders);
            }

            private static bool IsLink(FileSystemInfo entry) => entry.Attributes.HasFlag(FileAttributes.ReparsePoint);
        }
    }
}
