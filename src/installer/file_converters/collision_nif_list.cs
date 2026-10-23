using System.Text;

namespace AssetConverter
{
    /// <summary>
    /// The input of the collision worker's <c>--files-from</c>: the NIFs to consider, one path relative to the NIF source root per line,
    /// UTF-8 without a byte order mark, lines ended by a line feed (a file name cannot contain one). Either a file, deleted when this is
    /// disposed, or, with <see cref="StdinArgument"/>, the worker's stdin written by <see cref="StreamTo"/> while the NIFs are still arriving.
    /// </summary>
    internal sealed class CollisionNifList : IDisposable
    {
        /// <summary>The <c>--files-from</c> value that makes the worker read the list from its stdin until it is closed.</summary>
        public const string StdinArgument = "-";

        private static readonly UTF8Encoding Utf8 = new(encoderShouldEmitUTF8Identifier: false);

        private CollisionNifList(string filePath) => FilePath = filePath;

        public string FilePath { get; }

        /// <exception cref="ArgumentException">A file is not below <paramref name="sourceRoot"/>.</exception>
        public static CollisionNifList Create(string sourceRoot, IEnumerable<string> nifFiles)
        {
            var list = new CollisionNifList(Path.GetTempFileName());
            try
            {
                WriteTo(list.FilePath, sourceRoot, nifFiles);
                return list;
            }
            catch
            {
                list.Dispose();
                throw;
            }
        }

        internal static void WriteTo(string listPath, string sourceRoot, IEnumerable<string> nifFiles)
        {
            using var writer = new StreamWriter(listPath, append: false, Utf8) { NewLine = "\n" };
            foreach (string file in nifFiles)
                writer.WriteLine(RelativeToRoot(sourceRoot, file));
        }

        /// <summary>A writer of list lines onto <paramref name="stream"/>, the worker's stdin, which does not close the stream when it is disposed.</summary>
        public static StreamWriter OpenWriter(Stream stream) => new(stream, Utf8, bufferSize: 4096, leaveOpen: true) { NewLine = "\n" };

        /// <summary>
        /// Writes each file taken from <paramref name="candidates"/> to <paramref name="destination"/> the moment it is taken, flushed so the
        /// reader sees it at once, until the queue is finished and drained. <paramref name="onTaken"/> is called before each line is written.
        /// </summary>
        /// <returns>How many lines were written.</returns>
        /// <exception cref="ArgumentException">A file is not below <paramref name="sourceRoot"/>.</exception>
        /// <exception cref="OperationCanceledException"><paramref name="ct"/> was cancelled while waiting for the next file.</exception>
        /// <exception cref="Exception">The exception the queue failed with, or the one the destination threw (an <see cref="IOException"/> when its reader is gone).</exception>
        public static int StreamTo(TextWriter destination, string sourceRoot, NifWorkQueue candidates, Action? onTaken, CancellationToken ct)
        {
            int written = 0;
            while (candidates.TryTake(out string file, ct))
            {
                string relative = RelativeToRoot(sourceRoot, file);
                onTaken?.Invoke();
                destination.WriteLine(relative);
                destination.Flush();
                written++;
            }
            return written;
        }

        public void Dispose()
        {
            try
            {
                File.Delete(FilePath);
            }
            catch (Exception ex) when (ex is IOException or UnauthorizedAccessException)
            {
                Console.Error.WriteLine($"WARNING: could not delete the collision input list {FilePath}: {ex.Message}");
            }
        }

        internal static string RelativeToRoot(string sourceRoot, string file)
        {
            string relative = Path.GetRelativePath(sourceRoot, file);
            bool outside = relative == ".." || relative.StartsWith(".." + Path.DirectorySeparatorChar, StringComparison.Ordinal);
            if (outside || Path.IsPathRooted(relative))
                throw new ArgumentException($"'{file}' is not below the NIF source root '{sourceRoot}'.", nameof(file));
            return relative;
        }
    }
}
