namespace AssetConverter
{
    /// <summary>
    /// Publishes a conversion's output so that a reader, or the next run, only ever sees a complete one: it is built beside its place and
    /// renamed in, and a failure while publishing leaves the previous output where it was.
    /// </summary>
    internal static class AtomicPublish
    {
        public const string TemporarySuffix = ".tmp";
        private const string PreviousSuffix = ".old";

        /// <summary>Writes <paramref name="content"/> beside <paramref name="path"/> and renames it over the file.</summary>
        public static void File(string path, byte[] content) => Produced(path, temporary => System.IO.File.WriteAllBytes(temporary, content));

        /// <summary>Lets <paramref name="produce"/> write the file at a temporary path beside <paramref name="path"/> (any stale one is removed first) and renames it over the file once it returns.</summary>
        public static void Produced(string path, Action<string> produce)
        {
            Directory.CreateDirectory(Path.GetDirectoryName(Path.GetFullPath(path))!);
            string temporary = path + TemporarySuffix;
            try
            {
                Scratch.TryDeleteFile(temporary);
                produce(temporary);
                System.IO.File.Move(temporary, path, overwrite: true);
            }
            finally
            {
                Scratch.TryDeleteFile(temporary);
            }
        }

        public static void File(string path, string text) => File(path, System.Text.Encoding.UTF8.GetBytes(text));

        /// <summary>
        /// Lets <paramref name="populate"/> fill a sibling folder and swaps it for <paramref name="folder"/>: the previous folder is renamed
        /// aside first and put back when the swap fails, so the output is never lost to a failed publish.
        /// </summary>
        public static void Folder(string folder, Action<string> populate)
        {
            string temporary = folder + TemporarySuffix;
            string previous = folder + PreviousSuffix;
            bool published = false;
            try
            {
                Scratch.TryDeleteDir(temporary);
                Scratch.TryDeleteDir(previous);
                Directory.CreateDirectory(temporary);
                populate(temporary);

                bool hadPrevious = Directory.Exists(folder);
                if (hadPrevious)
                    Directory.Move(folder, previous);
                try
                {
                    Directory.Move(temporary, folder);
                    published = true;
                }
                catch
                {
                    if (hadPrevious && !Directory.Exists(folder))
                        Directory.Move(previous, folder);
                    throw;
                }
            }
            finally
            {
                Scratch.TryDeleteDir(temporary);
                if (published)
                    Scratch.TryDeleteDir(previous);
            }
        }
    }
}
