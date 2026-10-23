using System.Security.Cryptography;
using System.Text;

namespace AssetConverter.Audio
{
    /// <summary>
    /// How one source entry is turned into FLAC by ffmpeg: xWMA decodes to 16-bit samples; a WAV keeps its sample width, 16-bit as
    /// 16-bit and 24-bit as 24-bit (the FLAC encoder takes 24-bit samples as 32-bit words and the decoder of a 24-bit WAV declares the
    /// 24 significant bits). Sample rate and channel count stay as they are. Damaged input fails the run instead of yielding a partial
    /// file. The output carries no tags and no padding beyond the encoder vendor string that the FLAC muxer always writes.
    /// </summary>
    internal static class FlacEncoding
    {
        /// <summary>The encoder that converts each entry with <paramref name="ffmpeg"/>.</summary>
        public static AudioEncoder With(Ffmpeg ffmpeg) =>
            (entry, content, outputPath, ct) => ffmpeg.Run(Arguments(entry.Container, content, outputPath), content, ct);

        private static readonly string[] Quiet = ["-nostdin", "-v", "error"];
        private static readonly string[] Strict = ["-xerror", "-err_detect", "+explode+crccheck"];
        private static readonly string[] Plain = ["-map_metadata", "-1", "-bitexact"];
        private static readonly string[] Sixteen = ["-sample_fmt", "s16"];
        private static readonly string[] TwentyFour = ["-sample_fmt", "s32"];
        private static readonly string[] Encode = ["-c:a", "flac", "-metadata_header_padding", "0", "-f", "flac"];

        /// <summary>Stands for every argument of the conversion except the paths; part of the conversion stamp, so a change of arguments converts again.</summary>
        public static string SettingsFingerprint { get; } = Fingerprint();

        /// <summary>The hash of the argument lists of every kind of entry (xWMA, 16-bit WAV, 24-bit WAV) with an empty output path, one list per line.</summary>
        private static string Fingerprint()
        {
            var kinds = new (AudioContainer Container, string[] SampleFormat)[]
            {
                (AudioContainer.Xwma, Sixteen), (AudioContainer.Wav, Sixteen), (AudioContainer.Wav, TwentyFour),
            };
            string settings = string.Join('\n', kinds.Select(kind => string.Join(' ', Build(kind.Container, kind.SampleFormat, ""))));
            return Convert.ToHexString(SHA256.HashData(Encoding.UTF8.GetBytes(settings))).ToLowerInvariant();
        }

        /// <exception cref="InvalidDataException">A WAV is malformed or has a sample width other than 16 or 24 bits.</exception>
        public static IReadOnlyList<string> Arguments(AudioContainer container, ReadOnlySpan<byte> content, string outputPath)
        {
            string[] sampleFormat = container switch
            {
                AudioContainer.Xwma => Sixteen,
                AudioContainer.Wav => WavSampleFormat(WavFormat.BitsPerSample(content)),
                _ => throw new ArgumentOutOfRangeException(nameof(container), container, null),
            };
            return Build(container, sampleFormat, outputPath);
        }

        private static string[] Build(AudioContainer container, string[] sampleFormat, string outputPath) =>
        [
            .. Quiet, .. Strict, "-f", container == AudioContainer.Xwma ? "xwma" : "wav", "-i", "pipe:0",
            .. Plain, .. sampleFormat, .. Encode, outputPath,
        ];

        private static string[] WavSampleFormat(int bitsPerSample) => bitsPerSample switch
        {
            16 => Sixteen,
            24 => TwentyFour,
            _ => throw new InvalidDataException($"{bitsPerSample}-bit samples are neither 16-bit nor 24-bit"),
        };
    }
}
