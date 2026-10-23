namespace AssetConverter
{
    /// <summary>
    /// An unpacked payload folder that the running installer uses. It keeps the folder's completion marker open for reading, which other
    /// installers can see (they cannot open it exclusively) and which ends with the process, so a folder in use is never deleted as stale.
    /// </summary>
    internal sealed class PayloadFolder : IDisposable
    {
        private readonly FileStream _marker;

        private PayloadFolder(string path, FileStream marker)
        {
            Path = path;
            _marker = marker;
        }

        public string Path { get; }

        /// <exception cref="IOException">The completion marker cannot be opened.</exception>
        internal static PayloadFolder Hold(string path) =>
            new(path, new FileStream(System.IO.Path.Combine(path, PayloadExtractor.CompletionMarker), FileMode.Open, FileAccess.Read, FileShare.Read));

        /// <summary>Whether an installer holds the folder, judged by whether its completion marker can be opened exclusively; a folder without a marker is held by nobody.</summary>
        internal static bool IsHeld(string path)
        {
            string marker = System.IO.Path.Combine(path, PayloadExtractor.CompletionMarker);
            if (!File.Exists(marker)) return false;
            try
            {
                using var _ = new FileStream(marker, FileMode.Open, FileAccess.Read, FileShare.None);
                return false;
            }
            catch (IOException)
            {
                return true;
            }
        }

        public void Dispose() => _marker.Dispose();
    }
}
