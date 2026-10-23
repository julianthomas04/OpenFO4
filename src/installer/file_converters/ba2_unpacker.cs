using System.Diagnostics;
using SharpBSABA2.BA2Util;

namespace AssetConverter
{
    /// <summary>An archive could not be opened or one of its entries could not be extracted.</summary>
    public sealed class Ba2UnpackException : Exception
    {
        public string Archive { get; }

        public Ba2UnpackException(string archive, string stage, Exception inner)
            : base($"Failed to {stage} BA2 '{archive}': {inner.GetType().Name}: {inner.Message}", inner)
        {
            Archive = archive;
        }
    }

    public class BA2Unpacker
    {
        /// <exception cref="Ba2UnpackException">The archive cannot be opened or an entry fails to extract.</exception>
        public static void Unpack(string archive_dir, string target_dir)
        {
            BA2 sharp;
            try
            {
                sharp = new BA2(archive_dir);
            }
            catch (Exception ex) when (ex is IOException or InvalidDataException or UnauthorizedAccessException or ArgumentException or NotSupportedException)
            {
                throw new Ba2UnpackException(archive_dir, "open", ex);
            }

            try
            {
                List<SharpBSABA2.ArchiveEntry> files = sharp.Files;
                for (int i = 0; i < files.Count; i++)
                {
                    try
                    {
                        files[i].Extract(target_dir, true);
                    }
                    catch (Exception ex) when (ex is IOException or InvalidDataException or UnauthorizedAccessException or ArgumentException or NotSupportedException)
                    {
                        throw new Ba2UnpackException(archive_dir, $"extract entry {i}/{files.Count} of", ex);
                    }
                }
            }
            finally
            {
                sharp.Close();
            }
        }

        /// <summary>Unpacks every archive under <paramref name="archive_dir"/>, continuing past failed ones; returns the failures.</summary>
        public static IReadOnlyList<Ba2UnpackException> UnpackBatched(string archive_dir, string target_dir)
        {
            string[] files = Directory.GetFiles(archive_dir, "*.ba2", SearchOption.AllDirectories);
            Console.WriteLine($"Unpacking {files.Length} archives.");

            var failures = new List<Ba2UnpackException>();
            Stopwatch sw = new Stopwatch();
            sw.Start();

            for (int i = 0; i < files.Length; i++)
            {
                try
                {
                    Unpack(files[i], target_dir);
                }
                catch (Ba2UnpackException ex)
                {
                    failures.Add(ex);
                    Console.Error.WriteLine($"  ERROR: {ex.Message}");
                }
                Console.WriteLine($"  {i + 1}/{files.Length} ({(100.0 * (i + 1) / files.Length):F1}%) — {sw.Elapsed:mm\\:ss} elapsed");
            }

            sw.Stop();
            Console.WriteLine($"Done. {files.Length - failures.Count}/{files.Length} archives unpacked in {sw.Elapsed:mm\\:ss}, {failures.Count} failed.");
            return failures;
        }

        [Obsolete("Slower then UnpackBatched().")]
        public static async Task UnpackThreaded(string archive_dir, string target_dir)
        {
            string[] files = Directory.GetFiles(archive_dir, "*.ba2", SearchOption.AllDirectories);
            Console.WriteLine($"Unpacking {files.Length} archives.");
            Console.WriteLine($"Available CPU-Threads: {Environment.ProcessorCount}");

            int progress = 0;

            Stopwatch sw = new Stopwatch();
            sw.Start();

            Task[] tasks = new Task[files.Length];

            for (int i = 0; i < files.Length; i++)
            {
                int localIndex = i;
                tasks[i] = Task.Run(() =>
                {
                    try
                    {
                        Unpack(files[localIndex], target_dir);
                    }
                    catch (Ba2UnpackException ex)
                    {
                        Console.Error.WriteLine($"  ERROR: {ex.Message}");
                    }
                });
            }

            foreach (var task in tasks)
            {
                await task;

                progress += 1;
                Console.WriteLine($"  {progress}/{files.Length} ({(100.0 * progress / files.Length):F1}%) — {sw.Elapsed:mm\\:ss} elapsed");
            }

            sw.Stop();
            Console.WriteLine($"Done. {files.Length} archives unpacked in {sw.Elapsed:mm\\:ss}.");
        }

        //* Debug methods
        public static float GetLinuxUsedMemory()
        {
            var memInfo = File.ReadAllLines("/proc/meminfo");
            var total = ParseMemInfoLine(memInfo.First(l => l.StartsWith("MemTotal", StringComparison.Ordinal)));
            var available = ParseMemInfoLine(memInfo.First(l => l.StartsWith("MemAvailable", StringComparison.Ordinal)));

            float GB_available = (float)((total - available) / (1024.0 * 1024.0 * 1024.0));

            GB_available = MathF.Round(GB_available * 100f) / 100;

            return GB_available;
        }

        static long ParseMemInfoLine(string line)
        {
            // Remove non-digit characters, keeping only numbers
            string numbers = new string(line.Where(c => char.IsDigit(c)).ToArray());

            // Parse the number and convert from KB to bytes
            if (long.TryParse(numbers, out long memoryKB))
            {
                return memoryKB * 1024; // Convert KB to bytes
            }

            throw new FormatException($"Failed to parse memory info line: {line}");
        }
    }
}