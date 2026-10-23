using AssetConverter.Audio;
using Xunit;

namespace InstallerTests.Audio
{
    public sealed class WavFormatTests
    {
        [Theory]
        [InlineData(16, false)]
        [InlineData(24, false)]
        [InlineData(16, true)]
        [InlineData(24, true)]
        public void TheSampleWidthIsReadFromTheFormatChunkWhereverItSits(int bits, bool junkBeforeFormat)
        {
            Assert.Equal(bits, WavFormat.BitsPerSample(WavFile.Pcm(bits, junkBeforeFormat)));
        }

        [Fact]
        public void AnExtensiblePcmFormatIsPcm()
        {
            Assert.Equal(24, WavFormat.BitsPerSample(WavFile.Pcm(24, extensible: true)));
        }

        [Fact]
        public void ASampleFormatThatIsNotPcmIsRejected()
        {
            var adpcm = Assert.Throws<InvalidDataException>(() => WavFormat.BitsPerSample(WavFile.Pcm(4, formatTag: 2)));
            var extensibleFloat = Assert.Throws<InvalidDataException>(() => WavFormat.BitsPerSample(WavFile.Pcm(32, formatTag: 3, extensible: true)));

            Assert.Contains("not PCM", adpcm.Message, StringComparison.Ordinal);
            Assert.Contains("not PCM", extensibleFloat.Message, StringComparison.Ordinal);
        }

        [Fact]
        public void BytesThatAreNotAWaveFileAreRejected()
        {
            Assert.Throws<InvalidDataException>(() => WavFormat.BitsPerSample("RIFX0000WAVE"u8));
            Assert.Throws<InvalidDataException>(() => WavFormat.BitsPerSample([1, 2, 3]));
        }

        [Fact]
        public void AFileWithoutAFormatChunkOrWithATruncatedOneIsRejected()
        {
            byte[] wav = WavFile.Pcm(16);
            byte[] withoutFormat = [.. wav[..12], .. "data"u8, 0, 0, 0, 0];

            Assert.Throws<InvalidDataException>(() => WavFormat.BitsPerSample(withoutFormat));
            Assert.Throws<InvalidDataException>(() => WavFormat.BitsPerSample(wav.AsSpan(0, 30)));
        }
    }
}
