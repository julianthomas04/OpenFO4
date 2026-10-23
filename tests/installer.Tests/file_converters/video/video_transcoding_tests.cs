using System.Text;
using AssetConverter;
using AssetConverter.Audio;
using AssetConverter.Video;
using InstallerTests.Archives;
using Xunit;

namespace InstallerTests.Video
{
    /// <summary>The fake decoder and ffmpeg are real batch files, so the pipe between two real processes is what is tested.</summary>
    public sealed class VideoTranscodingTests : IDisposable
    {
        private const string InfoJson = """{"width":2,"height":2,"frameCount":3,"frameRateNumerator":24,"frameRateDenominator":1,"audioTracks":[{"id":0,"sampleRate":48000,"channels":2}]}""";
        private const int FrameBytes = 6;

        private static CancellationToken Ct => TestContext.Current.CancellationToken;

        private readonly ArchiveTempDir _dir = new();

        public void Dispose() => _dir.Dispose();

        private string Script(string name, params string[] lines) =>
            _dir.Write($"{name}-{Guid.NewGuid():N}.cmd", Encoding.ASCII.GetBytes("@echo off\r\n" + string.Join("\r\n", lines) + "\r\n"));

        /// <summary>A decoder whose <c>info</c> prints <paramref name="info"/> and whose <c>decode</c> runs <paramref name="decodeLines"/>.</summary>
        private Bink2Decoder Decoder(string info, params string[] decodeLines)
        {
            string infoFile = _dir.Write($"info-{Guid.NewGuid():N}.json", Encoding.ASCII.GetBytes(info));
            string[] lines = [$"if \"%1\"==\"info\" (type \"{infoFile}\" & exit /b 0)", .. decodeLines];
            return new Bink2Decoder(ToolCommand.Program(Script("fake-decoder", lines)));
        }

        private string Frames(int frames) => _dir.Write($"frames-{Guid.NewGuid():N}.bin", Enumerable.Repeat((byte)'A', frames * FrameBytes).ToArray());

        /// <summary>An ffmpeg that reads its whole stdin and reports <paramref name="frames"/> encoded frames.</summary>
        private Ffmpeg FfmpegReporting(int frames) =>
            new(ToolCommand.Program(Script("fake-ffmpeg", "more >nul", $"echo frame={frames}", $"echo out_time_us={AudioMicroseconds}", "echo progress=end", "exit /b 0")));

        /// <summary>What the fake ffmpeg measures for an audio track; the three frames of the fake video at 24 fps last 125000 microseconds.</summary>
        private long AudioMicroseconds { get; set; } = 125000;

        private void Transcode(Bink2Decoder decoder, Ffmpeg ffmpeg, CancellationToken ct) =>
            VideoTranscoding.With(decoder, ffmpeg)(Path.Combine(_dir.Path, "in.bk2"), Path.Combine(_dir.Path, "out.mp4.tmp"), ct);

        [Fact]
        public void ADecodedVideoStreamsThroughFfmpegAndComesOutWithAllItsFrames()
        {
            var decoder = Decoder(InfoJson, $"type \"{Frames(3)}\"", "exit /b 0");

            Transcode(decoder, FfmpegReporting(3), Ct);
        }

        [Fact]
        public void ADecoderThatFailsMidStreamFailsTheVideoWithItsExitCodeAndStderr()
        {
            var decoder = Decoder(InfoJson, $"type \"{Frames(1)}\"", "echo corrupt macroblock 1>&2", "exit /b 5");

            var error = Assert.Throws<FfmpegException>(() => Transcode(decoder, FfmpegReporting(3), Ct));

            Assert.Contains("exited 5", error.Message, StringComparison.Ordinal);
            Assert.Contains("corrupt macroblock", error.Message, StringComparison.Ordinal);
        }

        [Fact]
        public void AnFfmpegThatFailsFailsTheVideoWithItsStderrEvenWhileTheDecoderStillWrites()
        {
            string bulk = _dir.Write("bulk.bin", Enumerable.Repeat((byte)'A', 8 << 20).ToArray());
            var decoder = Decoder(InfoJson, $"type \"{bulk}\"", "exit /b 0");
            var ffmpeg = new Ffmpeg(ToolCommand.Program(Script("broken-ffmpeg", "echo no such encoder 1>&2", "exit /b 3")));

            var error = Assert.Throws<FfmpegException>(() => Transcode(decoder, ffmpeg, Ct));

            Assert.Contains("exited 3", error.Message, StringComparison.Ordinal);
            Assert.Contains("no such encoder", error.Message, StringComparison.Ordinal);
        }

        [Fact]
        public void ADecoderThatDeliversFewerFramesThanItAnnouncedFailsTheVideo()
        {
            var decoder = Decoder(InfoJson, $"type \"{Frames(2)}\"", "exit /b 0");

            var error = Assert.Throws<VideoConversionException>(() => Transcode(decoder, FfmpegReporting(2), Ct));

            Assert.Contains("fed 12", error.Message, StringComparison.Ordinal);
        }

        [Fact]
        public void AnFfmpegThatWroteOtherFramesThanTheDecoderAnnouncedFailsTheVideo()
        {
            var decoder = Decoder(InfoJson, $"type \"{Frames(3)}\"", "exit /b 0");

            var error = Assert.Throws<VideoConversionException>(() => Transcode(decoder, FfmpegReporting(2), Ct));

            Assert.Contains("wrote 2 frames", error.Message, StringComparison.Ordinal);
        }

        [Fact]
        public void AnAudioTrackOfAnotherDurationThanTheVideoFailsTheVideo()
        {
            var decoder = Decoder(InfoJson, $"type \"{Frames(3)}\"", "exit /b 0");
            AudioMicroseconds = 2_000_000;

            var error = Assert.Throws<VideoConversionException>(() => Transcode(decoder, FfmpegReporting(3), Ct));

            Assert.Contains("Audio track 0", error.Message, StringComparison.Ordinal);
        }

        [Fact]
        public void AnFfmpegThatStopsReadingWhileTheDecoderWritesNamesThatAsTheCauseNotAnEmptyFeed()
        {
            string bulk = _dir.Write("bulk2.bin", Enumerable.Repeat((byte)'A', 8 << 20).ToArray());
            var decoder = Decoder(InfoJson, $"type \"{bulk}\"", "exit /b 0");
            var ffmpeg = new Ffmpeg(ToolCommand.Program(Script("quitting-ffmpeg", "echo frame=3", "exit /b 0")));

            var error = Assert.Throws<FfmpegException>(() => Transcode(decoder, ffmpeg, Ct));

            Assert.Contains("stopped reading its input", error.Message, StringComparison.Ordinal);
        }

        [Fact]
        public void CancellingKillsAnFfmpegThatNeverReadsItsInputOrNoticesItsEnd()
        {
            var decoder = Decoder(InfoJson, $"type \"{Frames(3)}\"", "exit /b 0");
            var deaf = new Ffmpeg(ToolCommand.Program(Script("deaf-ffmpeg", "ping -n 60 127.0.0.1 >nul")));
            using var cancel = new CancellationTokenSource(TimeSpan.FromSeconds(2));
            var watch = System.Diagnostics.Stopwatch.StartNew();

            Assert.ThrowsAny<OperationCanceledException>(() => Transcode(decoder, deaf, cancel.Token));

            Assert.True(watch.Elapsed < TimeSpan.FromSeconds(30), $"took {watch.Elapsed}");
        }

        [Fact]
        public void ADecoderThatCannotReadTheFileFailsWithItsStderr()
        {
            string bad = Script("bad-info", "echo unsupported Bink2 revision 1>&2", "exit /b 2");
            var decoder = new Bink2Decoder(ToolCommand.Program(bad));

            var error = Assert.Throws<VideoConversionException>(() => Transcode(decoder, FfmpegReporting(0), Ct));

            Assert.Contains("unsupported Bink2 revision", error.Message, StringComparison.Ordinal);
            Assert.Contains("in.bk2", error.Message, StringComparison.Ordinal);
        }

        [Fact]
        public void CancellingKillsBothProcessesAndThrowsACancellation()
        {
            var decoder = Decoder(InfoJson, "ping -n 30 127.0.0.1 >nul");
            using var cancel = new CancellationTokenSource(TimeSpan.FromSeconds(2));
            var watch = System.Diagnostics.Stopwatch.StartNew();

            Assert.ThrowsAny<OperationCanceledException>(() => Transcode(decoder, FfmpegReporting(3), cancel.Token));

            Assert.True(watch.Elapsed < TimeSpan.FromSeconds(20), $"took {watch.Elapsed}");
        }

        [Fact]
        public void AMissingDecoderFailsNamingTheFile()
        {
            string missing = Path.Combine(_dir.Path, "no-decoder.exe");

            var error = Assert.Throws<VideoConversionException>(() => new Bink2Decoder(ToolCommand.Program(missing)).Info("a.bk2", Ct));

            Assert.Contains(missing, error.Message, StringComparison.Ordinal);
        }

        [Fact]
        public void TheDecoderVersionHashesEveryExecutableAndLibraryBesideTheProgram()
        {
            string exe = _dir.Write("decoder.exe", [1, 2, 3]);
            _dir.Write("decoder.dll", [4, 5, 6]);
            _dir.Write("notes.txt", [7]);
            string version = new Bink2Decoder(ToolCommand.Program(exe)).Version();

            _dir.Write("notes.txt", [8]);
            Assert.Equal(version, new Bink2Decoder(ToolCommand.Program(exe)).Version());
            _dir.Write("decoder.dll", [4, 5, 7]);
            Assert.NotEqual(version, new Bink2Decoder(ToolCommand.Program(exe)).Version());
            Assert.Matches("^sha256:[0-9a-f]{64}$", version);
        }
    }
}
