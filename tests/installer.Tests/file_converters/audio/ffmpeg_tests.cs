using AssetConverter;
using AssetConverter.Audio;
using InstallerTests.Archives;
using Xunit;

namespace InstallerTests.Audio
{
    public sealed class FfmpegTests : IDisposable
    {
        private static CancellationToken Ct => TestContext.Current.CancellationToken;

        private readonly ArchiveTempDir _dir = new();

        public void Dispose() => _dir.Dispose();

        /// <summary>A real process that stands in for ffmpeg: a batch file, which ignores its arguments and stdin.</summary>
        private Ffmpeg Tool(params string[] batchLines)
        {
            string path = _dir.Write($"fake-ffmpeg-{Guid.NewGuid():N}.cmd", System.Text.Encoding.ASCII.GetBytes("@echo off\r\n" + string.Join("\r\n", batchLines) + "\r\n"));
            return new Ffmpeg(ToolCommand.Program(path));
        }

        [Fact]
        public void ANonzeroExitFailsWithTheExitCodeAndStderrEvenWhenTheToolNeverReadItsInput()
        {
            var ffmpeg = Tool("echo broken stream 1>&2", "exit /b 3");

            var error = Assert.Throws<FfmpegException>(() => ffmpeg.Run(["-i", "pipe:0"], new byte[4 << 20], Ct));

            Assert.Contains("exited 3", error.Message, StringComparison.Ordinal);
            Assert.Contains("broken stream", error.Message, StringComparison.Ordinal);
        }

        [Fact]
        public void AZeroExitIsSuccess()
        {
            Tool("exit /b 0").Run([], [1, 2, 3], Ct);
        }

        [Fact]
        public void AZeroExitThatWarnedOnStderrIsSuccessBecauseDamagedInputIsMadeANonzeroExitByTheArguments()
        {
            var ffmpeg = Tool("echo [flac] Application provided invalid, non monotonically increasing dts to muxer in stream 0: 233984 >= 233856 1>&2", "exit /b 0");

            ffmpeg.Run([], [1, 2, 3], Ct);
        }

        [Fact]
        public void AStreamingRunReturnsWhatFfmpegWroteToStdoutAndHandsItEverythingTheProducerWrites()
        {
            var ffmpeg = Tool("more >nul", "echo frame=3");

            string stdout = ffmpeg.RunStreaming([], (stdin, _) => stdin.Write(Enumerable.Repeat((byte)0x41, 1 << 20).ToArray()), Ct);

            Assert.Equal("frame=3", stdout.Trim());
        }

        [Fact]
        public void AFailingProducerFailsAStreamingRunThatFfmpegFinishedWithZero()
        {
            var ffmpeg = Tool("more >nul", "exit /b 0");

            var error = Assert.Throws<FfmpegException>(() => ffmpeg.RunStreaming([], (_, _) => throw new InvalidOperationException("source broke"), Ct));

            Assert.Contains("source broke", error.Message, StringComparison.Ordinal);
        }

        [Fact]
        public void AStreamingRunThatFailedOnBothSidesReportsBothTexts()
        {
            var ffmpeg = Tool("echo encoder broke 1>&2", "exit /b 4");

            var error = Assert.Throws<FfmpegException>(() => ffmpeg.RunStreaming([], (_, _) => throw new InvalidOperationException("source broke"), Ct));

            Assert.Contains("exited 4", error.Message, StringComparison.Ordinal);
            Assert.Contains("encoder broke", error.Message, StringComparison.Ordinal);
            Assert.Contains("source broke", error.Message, StringComparison.Ordinal);
        }

        [Fact]
        public void StderrOfTheVersionQueryIsNotAFailure()
        {
            Assert.StartsWith("ffmpeg version x", Tool("echo ffmpeg version x", "echo noise 1>&2").Version(), StringComparison.Ordinal);
        }

        public static TheoryData<Exception, bool> InputFeedFailures => new()
        {
            { new IOException("The pipe is being closed."), true },
            { new ObjectDisposedException("stdin"), true },
            { new OperationCanceledException(), true },
            { new AggregateException(new IOException(), new AggregateException(new ObjectDisposedException("stdin"))), true },
            { new AggregateException(new IOException(), new UnauthorizedAccessException()), false },
            { new UnauthorizedAccessException(), false },
            { new InvalidOperationException(), false },
            { new NotSupportedException(), false },
        };

        [Theory]
        [MemberData(nameof(InputFeedFailures))]
        public void OnlyAStdinThatStoppedReadingIsAnExpectedFeedFailure(Exception error, bool expected)
        {
            Assert.Equal(expected, Ffmpeg.IsClosedInput(error));
        }

        [Fact]
        public void AMissingToolFailsNamingTheFile()
        {
            string missing = Path.Combine(_dir.Path, "no-ffmpeg.exe");
            var ffmpeg = new Ffmpeg(ToolCommand.Program(missing));

            var error = Assert.Throws<FfmpegException>(() => ffmpeg.Run([], [1], Ct));

            Assert.Contains(missing, error.Message, StringComparison.Ordinal);
        }

        [Fact]
        public void AProgramThatCannotBeLaunchedFailsExplicitly()
        {
            string notAProgram = _dir.Write("not-a-program.exe", [1, 2, 3, 4]);

            var error = Assert.Throws<FfmpegException>(() => new Ffmpeg(ToolCommand.Program(notAProgram)).Run([], [1], Ct));

            Assert.Contains("Could not launch ffmpeg", error.Message, StringComparison.Ordinal);
        }

        [Fact]
        public void TheVersionIsTheFirstLineOfTheToolsVersionOutputAndTheHashOfItsExecutable()
        {
            var ffmpeg = Tool("echo ffmpeg version librempeg-test Copyright", "echo second line");
            var other = Tool("echo ffmpeg version librempeg-test Copyright", "echo second line", "rem a different build");

            string version = ffmpeg.Version();

            Assert.StartsWith("ffmpeg version librempeg-test Copyright sha256:", version, StringComparison.Ordinal);
            Assert.DoesNotContain("second line", version, StringComparison.Ordinal);
            Assert.NotEqual(version, other.Version());
        }

        [Fact]
        public void CancellingKillsTheToolAndThrowsACancellation()
        {
            var ffmpeg = Tool("ping -n 30 127.0.0.1 >nul");
            using var cancel = new CancellationTokenSource(TimeSpan.FromMilliseconds(300));

            Assert.ThrowsAny<OperationCanceledException>(() => ffmpeg.Run([], [1], cancel.Token));
        }
    }
}
