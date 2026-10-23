using AssetConverter;
using AssetConverter.Audio;
using AssetConverter.Pipeline;
using InstallerTests.Archives;
using Xunit;

namespace InstallerTests.Audio
{
    /// <summary>Converts synthetic WAV files with the real Librempeg ffmpeg of the checkout (<c>build/librempeg</c>); skipped where it is not built.</summary>
    [Collection("Console redirection")]
    public sealed class RealFfmpegConversionTests : IDisposable
    {
        private const string Sounds = "Fallout4 - Sounds.ba2";
        private const string Startup = "Fallout4 - Startup.ba2";
        private const string Entry = @"Sound\fx\tone.wav";

        private static CancellationToken Ct => TestContext.Current.CancellationToken;

        private readonly ArchiveTempDir _game = new();
        private readonly ArchiveTempDir _install = new();

        public void Dispose()
        {
            _game.Dispose();
            _install.Dispose();
        }

        private string AudioOut => Path.Combine(_install.Path, "audio");

        private static Ffmpeg RequireFfmpeg()
        {
            var ffmpeg = Ffmpeg.Installed();
            try
            {
                ffmpeg.EnsureAvailable();
            }
            catch (FfmpegException missing)
            {
                Assert.Skip($"The real ffmpeg is not built: {missing.Message}");
            }
            return ffmpeg;
        }

        private void Convert(Ffmpeg ffmpeg, byte[] wav)
        {
            _game.Write(Sounds, Ba2Builder.Gnrl([new Ba2File(Entry, wav, Compress: false)]));
            _game.Write(Startup, Ba2Builder.Gnrl([]));
            using var conversion = AudioConversion.Plan(_game.Path, AudioOut, FlacEncoding.With(ffmpeg), "real", MemoryBudget.Unlimited(), workers: 1);
            conversion.Run(-1, Ct);
        }

        private string Output => Path.Combine(AudioOut, "sound", "fx", "tone.flac");

        private const int WavHeaderBytes = 44;
        private const int BytesPerFrame = 4;

        [Theory]
        [InlineData(0)]
        [InlineData(1)]
        public void ATruncatedWavFailsTheConversionNamingTheEntryAndLeavesNoFile(int bytesOfPartialFrame)
        {
            var ffmpeg = RequireFfmpeg();
            byte[] whole = WavFile.Pcm(16, frames: 10000);
            byte[] truncated = whole[..(WavHeaderBytes + 5000 * BytesPerFrame + bytesOfPartialFrame)];

            var error = Assert.Throws<AudioConversionException>(() => Convert(ffmpeg, truncated));

            Assert.Contains(Entry, error.Message, StringComparison.Ordinal);
            Assert.IsType<FfmpegException>(error.InnerException);
            Assert.False(File.Exists(Output));
            Assert.Empty(Directory.GetFiles(AudioOut, "*.tmp", SearchOption.AllDirectories));
        }

        [Theory]
        [InlineData(16)]
        [InlineData(24)]
        public void AWavKeepsItsSampleWidthRateAndChannelsAndCarriesOnlyTheVendorTag(int bits)
        {
            var ffmpeg = RequireFfmpeg();

            Convert(ffmpeg, WavFile.Pcm(bits, frames: 5000));

            var flac = FlacMetadata.Read(Output);
            Assert.Equal(bits, flac.BitsPerSample);
            Assert.Equal(48000, flac.SampleRate);
            Assert.Equal(2, flac.Channels);
            Assert.Equal([FlacMetadata.StreamInfo, FlacMetadata.VorbisComment], flac.Blocks.Select(block => block.Type));
            Assert.Equal("ffmpeg".Length + 8, flac.Blocks[1].Length);
        }
    }
}
