using System.Text;
using System.Text.Json;
using AssetConverter;
using AssetConverter.Audio;
using AssetConverter.Pipeline;
using AssetConverter.Video;
using InstallerTests.Archives;
using Xunit;

namespace InstallerTests.Pipeline
{
    /// <summary>The video stage composed with fake tools that are real batch files; prints to the console and reports to InstallProgress, so it runs serially with the tests that do.</summary>
    [Collection("Console redirection")]
    public sealed class PipelineStagesVideoTests : IDisposable
    {
        private const string InfoJson = """{"width":2,"height":2,"frameCount":3,"frameRateNumerator":24,"frameRateDenominator":1,"audioTracks":[]}""";

        private readonly ArchiveTempDir _game = new();
        private readonly ArchiveTempDir _install = new();
        private readonly ArchiveTempDir _tools = new();

        public void Dispose()
        {
            _game.Dispose();
            _install.Dispose();
            _tools.Dispose();
        }

        private string Script(string name, params string[] lines) =>
            _tools.Write($"{name}.cmd", Encoding.ASCII.GetBytes("@echo off\r\n" + string.Join("\r\n", lines) + "\r\n"));

        private Ffmpeg FakeFfmpeg(string versionLine) => new(ToolCommand.Program(Script($"ffmpeg-{Guid.NewGuid():N}",
            "setlocal enabledelayedexpansion",
            "set \"all=%*\"",
            $"if not \"!all:-version=!\"==\"!all!\" (echo {versionLine} & exit /b 0)",
            "if not \"!all:pipe:0=!\"==\"!all!\" goto encode",
            "echo out_time_us=125000",
            "exit /b 0",
            ":encode",
            "more >nul",
            "for %%a in (%*) do set \"last=%%~a\"",
            "echo x>\"!last!\"",
            "echo frame=3",
            "exit /b 0")));

        private Bink2Decoder FakeDecoder(string comment)
        {
            string info = _tools.Write($"info-{Guid.NewGuid():N}.json", Encoding.ASCII.GetBytes(InfoJson));
            string frames = _tools.Write($"frames-{Guid.NewGuid():N}.bin", Enumerable.Repeat((byte)'A', 18).ToArray());
            string folder = Path.Combine(_tools.Path, $"decoder-{Guid.NewGuid():N}");
            Directory.CreateDirectory(folder);
            File.WriteAllText(Path.Combine(folder, "decoder.dll"), comment);
            string program = Path.Combine(folder, "decoder.cmd");
            File.WriteAllText(program, string.Join("\r\n", "@echo off", $"if \"%1\"==\"info\" (type \"{info}\" & exit /b 0)", $"type \"{frames}\"", "exit /b 0"));
            File.WriteAllText(Path.Combine(folder, "decoder.exe"), comment);
            return new Bink2Decoder(ToolCommand.Program(program));
        }

        private PipelineStages Stages() => new(new InstallLocations(_game.Path, _install.Path), MemoryBudget.Unlimited(), CancellationToken.None);

        private string RecordedToolIdentity()
        {
            string stamp = Path.Combine(_install.Path, "assets", "video", "mainmenuloop.mp4.stamp");
            using var document = JsonDocument.Parse(File.ReadAllText(stamp));
            return document.RootElement.GetProperty("ffmpeg").GetString()!;
        }

        private void WriteGameVideo()
        {
            Directory.CreateDirectory(Path.Combine(_game.Path, "Data", "Video"));
            File.WriteAllBytes(Path.Combine(_game.Path, "Data", "Video", "MainMenuLoop.bk2"), TestBytes.Random(300, 1));
        }

        [Fact]
        public void TheStampNamesTheFfmpegTheDecoderAndTheEncodeSettings()
        {
            WriteGameVideo();
            var ffmpeg = FakeFfmpeg("ffmpeg version fake-1");
            var decoder = FakeDecoder("one");

            Stages().ConvertVideo(force: false, ffmpeg, decoder);

            Assert.True(File.Exists(Path.Combine(_install.Path, "assets", "video", "mainmenuloop.mp4")));
            Assert.Equal($"{ffmpeg.Version()} decoder:{decoder.Version()} arguments:{VideoEncoding.SettingsFingerprint}", RecordedToolIdentity());
            Assert.StartsWith("ffmpeg version fake-1 sha256:", RecordedToolIdentity(), StringComparison.Ordinal);
        }

        [Fact]
        public void ChangingTheDecoderOrTheFfmpegConvertsAgainWithoutForce()
        {
            WriteGameVideo();
            var ffmpeg = FakeFfmpeg("ffmpeg version fake-1");
            var decoder = FakeDecoder("one");
            Stages().ConvertVideo(force: false, ffmpeg, decoder);
            string first = RecordedToolIdentity();

            Stages().ConvertVideo(force: false, ffmpeg, decoder);
            Assert.Equal(first, RecordedToolIdentity());

            var rebuiltDecoder = FakeDecoder("two");
            Stages().ConvertVideo(force: false, ffmpeg, rebuiltDecoder);
            string second = RecordedToolIdentity();
            Assert.NotEqual(first, second);
            Assert.Contains(rebuiltDecoder.Version(), second, StringComparison.Ordinal);

            Stages().ConvertVideo(force: false, FakeFfmpeg("ffmpeg version fake-2"), rebuiltDecoder);
            Assert.Contains("fake-2", RecordedToolIdentity(), StringComparison.Ordinal);
        }
    }
}
