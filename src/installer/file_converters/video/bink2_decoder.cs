using System.Diagnostics;
using System.Security.Cryptography;
using System.Text;

namespace AssetConverter.Video
{
    /// <summary>
    /// The standalone Bink2 decoder (<c>tools/bink2-decoder</c>, GPL-3.0-or-later), run as a separate program and never linked
    /// into the installer: <c>info</c> states a video's properties as JSON, <c>decode</c> writes its frames as raw full-range planar
    /// 4:2:0 to stdout. Fails loudly with a <see cref="VideoConversionException"/>.
    /// </summary>
    internal sealed class Bink2Decoder(ToolCommand command)
    {
        public const string OverrideVariable = "OPENFO4_BINK2_DECODER";

        /// <summary>The decoder of the checkout's Release build or of the payload; <c>OPENFO4_BINK2_DECODER</c> names a program to use instead.</summary>
        public static Bink2Decoder Installed() =>
            new(Environment.GetEnvironmentVariable(OverrideVariable) is { Length: > 0 } program
                ? ToolCommand.Program(program)
                : InstallerPaths.Resources.Bink2Decoder());

        public void EnsureAvailable()
        {
            if (command.MissingFile() is { } missing)
                throw new VideoConversionException(
                    $"The Bink2 decoder was not found at '{missing}' — build it with 'dotnet build -c Release tools/bink2-decoder' (or set {OverrideVariable} to override).");
        }

        /// <summary>Identifies the decoder for the conversion stamp: the SHA-256 over the names and contents of every executable and library beside the program, so a rebuilt decoder converts again.</summary>
        public string Version()
        {
            EnsureAvailable();
            string folder = Path.GetDirectoryName(Path.GetFullPath(command.FileName))!;
            using var hash = IncrementalHash.CreateHash(HashAlgorithmName.SHA256);
            foreach (string file in Directory.EnumerateFiles(folder).Where(IsCode).Order(StringComparer.OrdinalIgnoreCase))
            {
                hash.AppendData(Encoding.UTF8.GetBytes(Path.GetFileName(file).ToLowerInvariant() + "\n"));
                using var content = File.OpenRead(file);
                byte[] buffer = new byte[1 << 16];
                for (int read; (read = content.Read(buffer)) > 0;) hash.AppendData(buffer.AsSpan(0, read));
            }
            return $"sha256:{Convert.ToHexString(hash.GetHashAndReset()).ToLowerInvariant()}";
        }

        private static bool IsCode(string file) => Path.GetExtension(file).ToLowerInvariant() is ".exe" or ".dll";

        /// <exception cref="VideoConversionException">The decoder cannot be started, fails on the file or does not state its properties as JSON.</exception>
        public VideoInfo Info(string videoPath, CancellationToken ct)
        {
            EnsureAvailable();
            using var process = Launch(["info", videoPath], videoPath);
            var stdout = process.StandardOutput.ReadToEndAsync(CancellationToken.None);
            var stderr = process.StandardError.ReadToEndAsync(CancellationToken.None);
            using var kill = ct.Register(() => ToolProcess.TryKill(process));
            process.WaitForExit();
            Task.WaitAll([stdout, stderr], CancellationToken.None);
            ct.ThrowIfCancellationRequested();
            if (process.ExitCode != 0)
                throw new VideoConversionException($"The Bink2 decoder exited {process.ExitCode} reading the properties of '{videoPath}'. {stderr.Result.Trim()}".Trim());
            return VideoInfo.Parse(stdout.Result);
        }

        /// <summary>Starts decoding the first <paramref name="frameCount"/> frames of the video; the caller reads <see cref="Decoding.Output"/> to its end and then calls <see cref="Decoding.EnsureSucceeded"/>.</summary>
        public Decoding Decode(string videoPath, int frameCount)
        {
            EnsureAvailable();
            return new Decoding(Launch(["decode", videoPath, "--frames", frameCount.ToString(System.Globalization.CultureInfo.InvariantCulture), "--output", "-"], videoPath), videoPath);
        }

        private Process Launch(IReadOnlyList<string> arguments, string videoPath)
        {
            var process = ToolProcess.Start(command, arguments, redirectInput: false, ex => new VideoConversionException(
                $"Could not launch the Bink2 decoder ('{command.FileName}', win32 err {ex.NativeErrorCode}) for '{videoPath}'; is the executable blocked?", ex));
            ToolProcess.LowerPriority(process);
            return process;
        }

        /// <summary>A running <c>decode</c>: its raw frames on <see cref="Output"/>, its stderr collected meanwhile so a full pipe never stalls it.</summary>
        internal sealed class Decoding : IDisposable
        {
            private readonly Process _process;
            private readonly string _videoPath;
            private readonly Task<string> _stderr;

            internal Decoding(Process process, string videoPath)
            {
                _process = process;
                _videoPath = videoPath;
                _stderr = process.StandardError.ReadToEndAsync(CancellationToken.None);
                Output = process.StandardOutput.BaseStream;
            }

            public Stream Output { get; }

            public void Kill() => ToolProcess.TryKill(_process);

            /// <exception cref="VideoConversionException">The decoder exited with an error; the message carries its stderr.</exception>
            public void EnsureSucceeded()
            {
                _process.WaitForExit();
                string errors = _stderr.GetAwaiter().GetResult().Trim();
                if (_process.ExitCode != 0)
                    throw new VideoConversionException($"The Bink2 decoder exited {_process.ExitCode} decoding '{_videoPath}'. {errors}".Trim());
            }

            public void Dispose()
            {
                if (!_process.HasExited) Kill();
                _process.WaitForExit();
                _process.Dispose();
            }
        }
    }
}
