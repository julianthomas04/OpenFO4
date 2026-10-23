using System.Security.Cryptography;

namespace AssetConverter.Audio
{
    /// <summary>
    /// Thin wrapper over the Librempeg ffmpeg (built by <c>tools/librempeg</c>), one process per call. Fails loudly with an
    /// <see cref="FfmpegException"/>: a missing tool, a launch failure, a nonzero exit (with its stderr) or a failure to hand it its input.
    /// Damaged input is made a nonzero exit by the arguments of the caller (<c>-xerror</c>); stderr of a run that exits 0 is not an error,
    /// the xWMA demuxer for one warns about the timestamp of its last packets while every sample is converted.
    /// </summary>
    internal sealed class Ffmpeg(ToolCommand command)
    {
        public const string OverrideVariable = "OPENFO4_FFMPEG";

        /// <summary>The ffmpeg of the checkout's build or of the payload; <c>OPENFO4_FFMPEG</c> names a program to use instead.</summary>
        public static Ffmpeg Installed() =>
            new(Environment.GetEnvironmentVariable(OverrideVariable) is { Length: > 0 } program
                ? ToolCommand.Program(program)
                : InstallerPaths.Resources.Ffmpeg());

        public void EnsureAvailable()
        {
            if (command.MissingFile() is { } missing)
                throw new FfmpegException($"ffmpeg not found at '{missing}' — build it with tools/librempeg (or set {OverrideVariable} to override).");
        }

        /// <summary>
        /// Identifies the tool for the conversion stamp: the version line it states and the hash of its executable. The hash is what
        /// catches a rebuild that adds or changes a codec without changing the version line.
        /// </summary>
        public string Version()
        {
            EnsureAvailable();
            string versionLine = Execute(["-version"], null, CancellationToken.None).Split('\n')[0].Trim();
            using var executable = File.OpenRead(command.FileName);
            return $"{versionLine} sha256:{Convert.ToHexString(SHA256.HashData(executable)).ToLowerInvariant()}";
        }

        /// <summary>Runs ffmpeg with <paramref name="input"/> on its stdin; cancelling <paramref name="ct"/> kills it and throws <see cref="OperationCanceledException"/>.</summary>
        public void Run(IReadOnlyList<string> arguments, byte[] input, CancellationToken ct) =>
            Execute(arguments, process => Feed(process, stdin => stdin.Write(input)), ct);

        /// <summary>
        /// Runs ffmpeg with whatever <paramref name="produce"/> writes to its stdin, without holding it in memory, and returns what ffmpeg wrote to
        /// stdout. A failure of <paramref name="produce"/> fails the run once ffmpeg has exited, the message carrying both sides' texts; cancelling
        /// <paramref name="ct"/> kills ffmpeg (the producer gets the token to stop its own source) and throws <see cref="OperationCanceledException"/>.
        /// </summary>
        public string RunStreaming(IReadOnlyList<string> arguments, Action<Stream, CancellationToken> produce, CancellationToken ct) =>
            Execute(arguments, process => Feed(process, stdin => produce(stdin, ct)), ct);

        /// <summary>Runs ffmpeg without input and returns what it wrote to stdout; a nonzero exit fails with its stderr.</summary>
        public string Capture(IReadOnlyList<string> arguments, CancellationToken ct) => Execute(arguments, null, ct);

        private string Execute(IReadOnlyList<string> arguments, Func<System.Diagnostics.Process, Exception?>? feeder, CancellationToken ct)
        {
            EnsureAvailable();
            // stdin is always ours: a run without input gets it closed at once, never the installer's own console input.
            using var process = ToolProcess.Start(command, arguments, redirectInput: true, ex => new FfmpegException(
                $"Could not launch ffmpeg ('{command.FileName}', win32 err {ex.NativeErrorCode}); is the executable blocked?", ex));
            ToolProcess.LowerPriority(process);

            var stdout = process.StandardOutput.ReadToEndAsync(CancellationToken.None);
            var stderr = process.StandardError.ReadToEndAsync(CancellationToken.None);
            var feed = Task.Run(() => feeder?.Invoke(process) ?? Feed(process, _ => { }), CancellationToken.None);
            using var kill = ct.Register(() => ToolProcess.TryKill(process));

            process.WaitForExit();
            Task.WaitAll([stdout, stderr, feed], CancellationToken.None);
            ct.ThrowIfCancellationRequested();
            string errors = stderr.Result.Trim();
            if (process.ExitCode != 0)
                throw new FfmpegException($"ffmpeg exited {process.ExitCode}. {errors}{(feed.Result is { } producerError ? $" The input side failed too: {producerError.Message}" : "")}".Trim());
            if (feed.Result is { } feedError)
                throw new FfmpegException($"Could not hand the input to ffmpeg: {feedError.Message}", feedError);
            return stdout.Result;
        }

        /// <summary>
        /// Lets <paramref name="write"/> write the input to ffmpeg's stdin. ffmpeg that fails early, or that has all it needs, closes its stdin and the write then fails
        /// too; the exit code and stderr tell that story, so such a failure is waited out until ffmpeg has exited and then dropped. Any
        /// other failure is returned.
        /// </summary>
        private static Exception? Feed(System.Diagnostics.Process process, Action<Stream> write)
        {
            Exception? failure = null;
            bool inputClosed = false;
            var stdin = process.StandardInput.BaseStream;
            try
            {
                write(stdin);
            }
            catch (Exception ex) when (IsClosedInput(ex))
            {
                inputClosed = true;
            }
            catch (Exception ex)
            {
                failure = ex;
            }
            try
            {
                // Closing flushes into the pipe, which fails when ffmpeg has stopped reading; that must not hide the producer's own failure.
                stdin.Dispose();
            }
            catch (Exception ex) when (IsClosedInput(ex))
            {
                inputClosed = true;
            }
            catch (Exception ex)
            {
                failure ??= ex;
            }
            if (failure is null && inputClosed) process.WaitForExit();
            return failure;
        }

        /// <summary>Whether <paramref name="error"/> is what writing to a process that stopped reading looks like: a broken pipe, a stdin already disposed or a cancelled write; an aggregate counts when every part does.</summary>
        internal static bool IsClosedInput(Exception error) => error switch
        {
            AggregateException aggregate => aggregate.Flatten().InnerExceptions.All(IsClosedInput),
            IOException or ObjectDisposedException or OperationCanceledException => true,
            _ => false,
        };
    }
}
