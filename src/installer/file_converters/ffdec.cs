using System.Diagnostics;
using System.Text.RegularExpressions;
using AssetConverter.Pipeline;

namespace AssetConverter
{
    /// Thin wrapper over the vendored JPEXS ffdec CLI (a Java tool). Shared by
    /// the font and Pip-Boy art extractors. Fails loudly with an <see cref="FfdecException"/>:
    /// a missing tool, a launch failure (missing JRE / blocked exe), a timeout
    /// (hung JVM), or a nonzero exit, each with a cause-specific message.
    public static partial class Ffdec
    {
        /// The vendored ffdec (third_party/ffdec) on the system's or the bundled Java; `OPENFO4_FFDEC` names a launcher to use instead.
        static ToolCommand Command =>
            Environment.GetEnvironmentVariable("OPENFO4_FFDEC") is { Length: > 0 } launcher
                ? ToolCommand.Program(launcher)
                : InstallerPaths.Resources.Ffdec();

        /// How long ffdec may go without any sign of work (output on stdout or stderr, a new or changed file in the export folder)
        /// before it counts as hung. The run as a whole is not limited: a large export on a loaded machine takes minutes but keeps
        /// producing files every few seconds, while a hung JVM produces nothing. One symbol's frames render in seconds, so two minutes
        /// of silence leaves a wide margin and still ends a genuine hang quickly.
        public static readonly TimeSpan StallLimit = TimeSpan.FromMinutes(2);

        /// How long ffdec may take to show its first sign of work: JVM start and parsing a large movie come first, and on a loaded
        /// machine they take longer than the gap between later signs.
        public static readonly TimeSpan StartupLimit = TimeSpan.FromMinutes(4);

        /// How often progress is checked: small against <see cref="StallLimit"/>, large enough that scanning the export folder stays cheap.
        static readonly TimeSpan PollInterval = TimeSpan.FromSeconds(2);

        [GeneratedRegex(@"^JPEXS Free Flash Decompiler v\.(?<version>\S+)", RegexOptions.CultureInvariant)]
        private static partial Regex VersionLine();

        public static void EnsureAvailable() => EnsureAvailable(Command);

        static void EnsureAvailable(ToolCommand command)
        {
            if (command.MissingFile() is { } missing)
                throw new FfdecException($"ffdec not found at '{missing}' — vendored tool missing (set OPENFO4_FFDEC to override).");
        }

        /// What the `-help` probe reserves: the JVM only starts, prints and exits without exporting anything. Measured peak working set is
        /// about 60 MiB; this leaves four times that for a slower start or a different JRE.
        internal const long VersionProbeBytes = 256L * 1024 * 1024;

        /// The version the tool states about itself (`-help` opens with it), which outputs converted through it are stamped with.
        public static string Version(MemoryBudget memory, CancellationToken ct) => ParseVersion(Execute(memory, VersionProbeBytes, null, ["-help"], ct));

        /// The version a `-help` output states.
        public static string ParseVersion(string helpOutput) =>
            VersionLine().Match(helpOutput.TrimStart()) is { Success: true } line
                ? line.Groups["version"].Value
                : throw new FfdecException($"ffdec does not state its version: its help output opens with '{helpOutput.TrimStart().Split('\n')[0].Trim()}'.");

        /// Run `ffdec-cli <args...>` for an export into <paramref name="outputDirectory"/>, whose growing content shows that ffdec is
        /// working. Throws on launch failure, a stall, or a nonzero exit (with both stdout and stderr: JPEXS writes diagnostics to either).
        public static void RunExporting(MemoryBudget memory, string outputDirectory, string[] args, CancellationToken ct) =>
            Execute(memory, MemoryEstimates.FfdecProcessBytes, outputDirectory, args, ct);

        static string Execute(MemoryBudget memory, long reservedBytes, string? outputDirectory, string[] args, CancellationToken ct)
        {
            var command = Command;
            EnsureAvailable(command);
            using var reservation = memory.Reserve(reservedBytes, ct);
            using var covered = reservation.CoverChildProcesses();
            using var p = ToolProcess.Start(command, args, redirectInput: false, ex => new FfdecException(
                $"Could not launch ffdec ('{command.FileName}', win32 err {ex.NativeErrorCode}). ffdec requires " +
                "a Java runtime it can find and an unblocked executable.", ex));

            var output = new CapturedOutput();
            p.OutputDataReceived += (_, e) => output.AddStdout(e.Data);
            p.ErrorDataReceived += (_, e) => output.AddStderr(e.Data);
            p.BeginOutputReadLine();
            p.BeginErrorReadLine();

            var clock = Stopwatch.StartNew();
            var watchdog = new StallWatchdog(StallLimit, StartupLimit, () => clock.Elapsed);
            bool exited;
            try
            {
                exited = watchdog.WaitForExit(new WatchedFfdec(p, output, outputDirectory), PollInterval);
            }
            catch
            {
                ToolProcess.TryKill(p);
                throw;
            }

            if (!exited)
                throw new FfdecException(
                    $"ffdec made no progress for {watchdog.ActiveLimit.TotalSeconds:F0}s ({(watchdog.HasStarted ? "stall limit, after its first output" : "startup limit, before any output")}; no output, no export file changes; hung JRE?) after running {clock.Elapsed.TotalSeconds:F0}s; process {ToolProcess.TryKill(p)}.");

            p.WaitForExit();
            string stdout = output.Stdout;
            string stderr = output.Stderr;
            if (p.ExitCode != 0)
                throw new FfdecException($"ffdec exited {p.ExitCode}. {stderr.Trim()} {stdout.Trim()}".Trim());
            return stdout;
        }

        sealed class CapturedOutput
        {
            readonly object _gate = new();
            readonly System.Text.StringBuilder _stdout = new(), _stderr = new();
            long _chars;

            public void AddStdout(string? line) => Add(_stdout, line);
            public void AddStderr(string? line) => Add(_stderr, line);

            void Add(System.Text.StringBuilder target, string? line)
            {
                if (line is null) return;
                lock (_gate)
                {
                    target.AppendLine(line);
                    _chars += line.Length + 1;
                }
            }

            public long Chars { get { lock (_gate) return _chars; } }
            public string Stdout { get { lock (_gate) return _stdout.ToString(); } }
            public string Stderr { get { lock (_gate) return _stderr.ToString(); } }
        }

        /// The progress of an ffdec process: the output it wrote plus the files and bytes in its export folder.
        sealed class WatchedFfdec(Process process, CapturedOutput output, string? outputDirectory) : IWatchedProcess
        {
            public bool WaitForExit(TimeSpan timeout) => process.WaitForExit(timeout);

            long _lastExportedSize;

            public long Progress() => output.Chars + ExportedSize();

            long ExportedSize()
            {
                if (outputDirectory is null || !Directory.Exists(outputDirectory)) return 0;
                long total = 0;
                try
                {
                    foreach (var file in new DirectoryInfo(outputDirectory).EnumerateFiles("*", SearchOption.AllDirectories))
                        total += 1 + file.Length;
                }
                catch (Exception e) when (e is IOException or UnauthorizedAccessException)
                {
                    return _lastExportedSize;
                }
                return _lastExportedSize = total;
            }
        }
    }
}
