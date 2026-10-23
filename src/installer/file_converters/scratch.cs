using System.IO;

namespace AssetConverter
{
    /// Best-effort cleanup of scratch files/dirs. Logs the expected I/O failures
    /// (locked/permission) rather than swallowing them, and lets truly
    /// unexpected exceptions propagate.
    public static class Scratch
    {
        public static void TryDeleteFile(string path)
        {
            try
            {
                if (File.Exists(path))
                    File.Delete(path);
            }
            catch (Exception e) when (e is IOException or UnauthorizedAccessException)
            {
                Console.Error.WriteLine($"Cleanup: could not delete '{path}': {e.Message}");
            }
        }

        public static void TryDeleteDir(string dir)
        {
            try
            {
                if (Directory.Exists(dir))
                    Directory.Delete(dir, recursive: true);
            }
            catch (Exception e) when (e is IOException or UnauthorizedAccessException)
            {
                Console.Error.WriteLine($"Cleanup: could not delete '{dir}': {e.Message}");
            }
        }
    }
}
