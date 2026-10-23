using System.Diagnostics;
using AssetConverter.Progress;

namespace AssetConverter.Pipeline
{
    /// <summary>The end-of-run report: total time, the overlapping phase timeline, per-step durations, the size of the output and the peak memory of the installer process.</summary>
    internal static class InstallSummary
    {
        public static void Print(PhaseLog phases, ProgressSnapshot steps, InstallLocations paths)
        {
            var glbs = Tally(paths.MeshesOut, "*.glb");
            var cols = Tally(paths.MeshesOut, "*.col.json");
            var dds = Tally(paths.TexturesOut, "*.dds");
            var raw = Tally(paths.StagingRoot, "*");
            long dbBytes = File.Exists(paths.DatabasePath) ? new FileInfo(paths.DatabasePath).Length : 0;

            Console.WriteLine();
            Console.WriteLine("══════════════════════════════════════════════════════════════");
            Console.WriteLine("  INSTALL COMPLETE");
            Console.WriteLine("══════════════════════════════════════════════════════════════");
            Console.WriteLine($"  Total time   {ProgressFormatter.Invariant.Duration(phases.Elapsed)}");
            Console.WriteLine($"  Peak memory  {ByteFormat.Format(PeakWorkingSetBytes())}  (working set of this process)");
            Console.WriteLine();
            Console.WriteLine("  Phase timeline (phases overlap; offsets from start):");
            foreach (var (name, start, end) in phases.Entries)
                Console.WriteLine($"    {ProgressFormatter.Invariant.Duration(start),9} -> {ProgressFormatter.Invariant.Duration(end),9}  ({ProgressFormatter.Invariant.Duration(end - start),9})  {name}");
            Console.WriteLine();
            Console.WriteLine("  Steps:");
            foreach (var step in steps.Steps)
            {
                PrintStep(step, indent: "");
                foreach (var part in step.Parts) PrintStep(part, indent: "  ");
            }
            Console.WriteLine();
            Console.WriteLine("  Output:");
            Console.WriteLine($"    Meshes (.glb)      {glbs.Count,6:N0} files   {ByteFormat.Format(glbs.Bytes)}");
            Console.WriteLine($"    Collision (.col)   {cols.Count,6:N0} files");
            Console.WriteLine($"    Textures (.dds)    {dds.Count,6:N0} files   {ByteFormat.Format(dds.Bytes)}");
            Console.WriteLine($"    Database           {ByteFormat.Format(dbBytes)}");
            Console.WriteLine(Directory.Exists(paths.StagingRoot)
                ? $"    Raw staging        {raw.Count,6:N0} files   {ByteFormat.Format(raw.Bytes)}"
                : "    Raw staging        removed");
            Console.WriteLine("══════════════════════════════════════════════════════════════");
        }

        private static void PrintStep(StepSnapshot step, string indent) =>
            Console.WriteLine($"    {ProgressFormatter.Invariant.Duration(step.Elapsed),9}  {indent + step.Name,-34}  {ProgressFormatter.Invariant.Detailed(step)}");

        private static long PeakWorkingSetBytes()
        {
            using var process = Process.GetCurrentProcess();
            return process.PeakWorkingSet64;
        }

        // DirectoryInfo enumeration: FileInfos arrive pre-populated from the directory walk (no per-file stat storm).
        private static (int Count, long Bytes) Tally(string root, string pattern)
        {
            if (!Directory.Exists(root)) return (0, 0);
            int count = 0;
            long bytes = 0;
            foreach (var file in new DirectoryInfo(root).EnumerateFiles(pattern, SearchOption.AllDirectories))
            {
                count++;
                bytes += file.Length;
            }
            return (count, bytes);
        }
    }
}
