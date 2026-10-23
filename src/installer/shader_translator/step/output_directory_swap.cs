namespace AssetConverter.ShaderTranslation.Installation
{
    /// <summary>
    /// Exclusive ownership of the shader output folder for the duration of a run, so that a second installer cannot recover, replace or delete the
    /// first one's temporary directories. The lock is an open file handle (<see cref="FileShare.None"/>) on a file next to the output
    /// folder: the OS releases it when the owner exits or dies, so a lock file left behind by a killed run is never stale and never
    /// has to be removed by hand.
    /// </summary>
    internal sealed class ShaderOutputLock : IDisposable
    {
        public const string FileSuffix = ".lock";
        private const int ErrorSharingViolation = 32;
        private const int ErrorLockViolation = 33;

        private readonly FileStream _handle;

        private ShaderOutputLock(string path, FileStream handle)
        {
            LockPath = path;
            _handle = handle;
        }

        public string LockPath { get; }

        public static string PathFor(string stem) => stem + FileSuffix;

        /// <exception cref="ShaderOutputLockedException">Another run holds the lock.</exception>
        /// <exception cref="ShaderTranslateStepException">The lock file cannot be created for another reason (permissions, a directory in the way).</exception>
        public static ShaderOutputLock Acquire(string stem)
        {
            string path = PathFor(stem);
            try
            {
                var handle = new FileStream(path, FileMode.OpenOrCreate, FileAccess.ReadWrite, FileShare.None, bufferSize: 1, FileOptions.DeleteOnClose);
                return new ShaderOutputLock(path, handle);
            }
            catch (IOException ex) when (IsSharingViolation(ex))
            {
                throw new ShaderOutputLockedException(path, ex);
            }
            catch (Exception ex) when (ex is IOException or UnauthorizedAccessException)
            {
                throw new ShaderTranslateStepException($"Cannot create the lock file {path} that guards the shaders folder: {ex.Message}", ex);
            }
        }

        public void Dispose() => _handle.Dispose();

        private static bool IsSharingViolation(IOException ex) => (ex.HResult & 0xFFFF) is ErrorSharingViolation or ErrorLockViolation;
    }

    /// <summary>The directory operations the output swap needs; tests substitute failing implementations.</summary>
    internal class DirectoryOperations
    {
        public virtual bool Exists(string path) => Directory.Exists(path);

        public virtual void Move(string source, string destination) => Directory.Move(source, destination);

        public virtual void DeleteRecursive(string path) => Directory.Delete(path, recursive: true);

        public virtual IEnumerable<string> EnumerateDirectories(string parent, string pattern) => Directory.EnumerateDirectories(parent, pattern);

        public virtual DateTime LastWriteTimeUtc(string path) => Directory.GetLastWriteTimeUtc(path);

        public virtual void Sleep(TimeSpan duration) => Thread.Sleep(duration);
    }

    /// <summary>
    /// Replaces the shaders folder with a freshly written one. Windows refuses to rename a directory while any handle inside it
    /// is open (the running game, virus scanners, the search indexer), so renames are retried with backoff. A previous shaders folder is only
    /// discarded after its replacement is live, and a backup left behind by a killed run is restored rather than deleted.
    /// </summary>
    internal sealed class OutputDirectorySwap(DirectoryOperations operations, Action<string> warn)
    {
        public const string TempMarker = ".tmp-";
        public const string BackupMarker = ".old-";
        public const int MaxAttempts = 5;
        public static readonly TimeSpan InitialBackoff = TimeSpan.FromMilliseconds(100);

        public static string NewTempPath(string stem) => $"{stem}{TempMarker}{Guid.NewGuid():N}";

        private static string NewBackupPath(string stem) => $"{stem}{BackupMarker}{Guid.NewGuid():N}";

        /// <summary>Brings the shaders folder into a consistent state after an earlier run was killed mid-way.</summary>
        public void RecoverFromKilledRun(string stem)
        {
            string? parent = Path.GetDirectoryName(stem);
            if (parent == null || !operations.Exists(parent)) return;
            string name = Path.GetFileName(stem);

            var temps = new List<string>();
            var backups = new List<string>();
            foreach (string dir in operations.EnumerateDirectories(parent, name + ".*"))
            {
                string suffix = Path.GetFileName(dir)[name.Length..];
                if (suffix.StartsWith(TempMarker, StringComparison.Ordinal)) temps.Add(dir);
                else if (suffix.StartsWith(BackupMarker, StringComparison.Ordinal)) backups.Add(dir);
            }

            if (!operations.Exists(stem) && backups.Count > 0)
            {
                string newest = backups.OrderByDescending(operations.LastWriteTimeUtc).First();
                Retry(() => operations.Move(newest, stem), $"restoring the previous shaders folder from {newest}");
                backups.Remove(newest);
            }

            foreach (string leftover in temps.Concat(backups))
                DiscardQuietly(leftover);
        }

        /// <summary>Swaps <paramref name="tempDir"/> into <paramref name="finalDir"/>; on failure the previous shaders folder is put back.</summary>
        public void Publish(string tempDir, string finalDir, string stem)
        {
            string? backup = null;
            if (operations.Exists(finalDir))
            {
                backup = NewBackupPath(stem);
                Retry(() => operations.Move(finalDir, backup), $"moving the existing shaders folder {finalDir} aside");
            }

            try
            {
                Retry(() => operations.Move(tempDir, finalDir), $"moving the new shaders folder into {finalDir}");
            }
            catch (Exception swapFailure)
            {
                if (backup != null) Restore(backup, finalDir, swapFailure);
                throw;
            }

            if (backup != null) DiscardQuietly(backup);
        }

        public void DiscardQuietly(string directory)
        {
            if (!operations.Exists(directory)) return;
            try
            {
                operations.DeleteRecursive(directory);
            }
            catch (Exception ex) when (ex is IOException or UnauthorizedAccessException)
            {
                warn($"WARNING: could not remove temporary directory {directory}: {ex.Message}");
            }
        }

        private void Restore(string backup, string finalDir, Exception swapFailure)
        {
            try
            {
                Retry(() => operations.Move(backup, finalDir), $"restoring the previous shaders folder from {backup}");
            }
            catch (Exception restoreFailure)
            {
                throw new ShaderTranslateStepException(
                    $"Publishing the shaders folder failed and the previous one could not be restored; it is preserved in {backup} (rename it to {finalDir}).",
                    new AggregateException(swapFailure, restoreFailure));
            }
        }

        private void Retry(Action action, string description)
        {
            var delay = InitialBackoff;
            for (int attempt = 1; ; attempt++)
            {
                try
                {
                    action();
                    return;
                }
                catch (Exception ex) when ((ex is IOException or UnauthorizedAccessException) && ex is not (DirectoryNotFoundException or FileNotFoundException))
                {
                    if (attempt == MaxAttempts)
                        throw new ShaderTranslateStepException(
                            $"Failed {description} after {MaxAttempts} attempts: {ex.Message} Close the game and any program that has files of the shaders folder open, then run shader-translate again.", ex);
                    operations.Sleep(delay);
                    delay *= 2;
                }
            }
        }
    }
}
