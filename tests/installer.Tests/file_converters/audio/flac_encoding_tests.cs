using AssetConverter.Audio;
using Xunit;

namespace InstallerTests.Audio
{
    public sealed class FlacEncodingTests
    {
        private const string Output = "out.flac.tmp";

        private static string[] Arguments(AudioContainer container, byte[] content) => [.. FlacEncoding.Arguments(container, content, Output)];

        [Fact]
        public void XwmaIsDecodedToSixteenBitSamples()
        {
            Assert.Equal(
                [
                    "-nostdin", "-v", "error", "-xerror", "-err_detect", "+explode+crccheck", "-f", "xwma", "-i", "pipe:0",
                    "-map_metadata", "-1", "-bitexact", "-sample_fmt", "s16", "-c:a", "flac", "-metadata_header_padding", "0", "-f", "flac", Output,
                ],
                Arguments(AudioContainer.Xwma, [1, 2, 3]));
        }

        [Theory]
        [InlineData(false)]
        [InlineData(true)]
        public void ASixteenBitWavStaysSixteenBitWhetherOrNotJunkPrecedesTheFormatChunk(bool junk)
        {
            string[] arguments = Arguments(AudioContainer.Wav, WavFile.Pcm(16, junk));

            Assert.Equal(["-f", "wav", "-i", "pipe:0"], arguments[6..10]);
            Assert.Equal(["-sample_fmt", "s16", "-c:a", "flac"], arguments[13..17]);
        }

        [Theory]
        [InlineData(false)]
        [InlineData(true)]
        public void ATwentyFourBitWavStaysTwentyFourBitWhetherOrNotJunkPrecedesTheFormatChunk(bool junk)
        {
            string[] arguments = Arguments(AudioContainer.Wav, WavFile.Pcm(24, junk));

            Assert.Equal(["-sample_fmt", "s32", "-c:a", "flac"], arguments[13..17]);
            Assert.Equal(Output, arguments[^1]);
        }

        [Fact]
        public void DamagedInputFailsTheRunInsteadOfYieldingAPartialFile()
        {
            string[] arguments = Arguments(AudioContainer.Wav, WavFile.Pcm(16));

            Assert.Contains("-xerror", arguments);
            Assert.Equal("+explode+crccheck", arguments[Array.IndexOf(arguments, "-err_detect") + 1]);
            Assert.True(Array.IndexOf(arguments, "-err_detect") < Array.IndexOf(arguments, "-i"), "an input option must precede the input");
        }

        [Fact]
        public void TheFlacMuxerIsToldToWriteNoPaddingBlock()
        {
            string[] arguments = Arguments(AudioContainer.Xwma, [1]);

            Assert.Equal("0", arguments[Array.IndexOf(arguments, "-metadata_header_padding") + 1]);
            Assert.True(Array.IndexOf(arguments, "-metadata_header_padding") > Array.IndexOf(arguments, "-i"), "an output option must follow the input");
        }

        [Fact]
        public void TheSettingsFingerprintHashesTheArgumentsOfEveryKindOfEntry()
        {
            string settings = string.Join('\n', new[]
            {
                FlacEncoding.Arguments(AudioContainer.Xwma, [1], ""), FlacEncoding.Arguments(AudioContainer.Wav, WavFile.Pcm(16), ""), FlacEncoding.Arguments(AudioContainer.Wav, WavFile.Pcm(24), ""),
            }.Select(arguments => string.Join(' ', arguments)));

            string expected = Convert.ToHexString(System.Security.Cryptography.SHA256.HashData(System.Text.Encoding.UTF8.GetBytes(settings))).ToLowerInvariant();

            Assert.Equal(expected, FlacEncoding.SettingsFingerprint);
        }

        [Fact]
        public void AWavWithAnotherSampleWidthIsRejected()
        {
            var error = Assert.Throws<InvalidDataException>(() => FlacEncoding.Arguments(AudioContainer.Wav, WavFile.Pcm(8), Output));

            Assert.Contains("8-bit", error.Message, StringComparison.Ordinal);
        }
    }
}
