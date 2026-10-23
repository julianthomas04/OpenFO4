using System.Buffers.Binary;

namespace AssetConverter.Audio
{
    /// <summary>Reads what the conversion needs from a RIFF WAVE file: the sample width of its PCM data, found by walking the chunks (files carry a <c>JUNK</c> chunk before <c>fmt </c>).</summary>
    internal static class WavFormat
    {
        private const ushort PcmTag = 1;
        private const ushort ExtensibleTag = 0xFFFE;
        private const int RiffHeaderSize = 12;
        private const int ChunkHeaderSize = 8;
        private const int FmtBitsPerSampleOffset = 14;
        private const int FmtMinimumSize = 16;
        private const int ExtensibleSubFormatOffset = 24;

        /// <exception cref="InvalidDataException">Not a WAVE file, no <c>fmt </c> chunk, or the samples are not PCM.</exception>
        public static int BitsPerSample(ReadOnlySpan<byte> wav)
        {
            if (wav.Length < RiffHeaderSize || !wav[..4].SequenceEqual("RIFF"u8) || !wav.Slice(8, 4).SequenceEqual("WAVE"u8))
                throw new InvalidDataException("not a RIFF WAVE file");

            int position = RiffHeaderSize;
            while (position + ChunkHeaderSize <= wav.Length)
            {
                ReadOnlySpan<byte> id = wav.Slice(position, 4);
                uint size = BinaryPrimitives.ReadUInt32LittleEndian(wav.Slice(position + 4, 4));
                int body = position + ChunkHeaderSize;
                if (id.SequenceEqual("fmt "u8))
                    return ReadFormat(wav, body, size);
                long next = (long)body + size + (size & 1);
                if (next > wav.Length) break;
                position = (int)next;
            }
            throw new InvalidDataException("no fmt chunk");
        }

        private static int ReadFormat(ReadOnlySpan<byte> wav, int body, uint size)
        {
            if (size < FmtMinimumSize || body + size > wav.Length)
                throw new InvalidDataException("truncated fmt chunk");
            ushort tag = BinaryPrimitives.ReadUInt16LittleEndian(wav.Slice(body, 2));
            if (tag == ExtensibleTag && size >= ExtensibleSubFormatOffset + 2)
                tag = BinaryPrimitives.ReadUInt16LittleEndian(wav.Slice(body + ExtensibleSubFormatOffset, 2));
            if (tag != PcmTag)
                throw new InvalidDataException($"sample format tag {tag} is not PCM");
            return BinaryPrimitives.ReadUInt16LittleEndian(wav.Slice(body + FmtBitsPerSampleOffset, 2));
        }
    }
}
