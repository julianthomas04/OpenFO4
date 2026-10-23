using System.Buffers.Binary;

namespace AssetConverter
{
    /// Makes an exported TrueType font byte-reproducible by clearing the
    /// `head` table's created/modified timestamps to 0 ("no date"; the fields
    /// are mandatory, ffdec stamps the export time) and recomputing the checksums those bytes feed into. Nothing
    /// reads the timestamps: the engine's font stack (ttf-parser) skips them.
    public static class TtfNormalizer
    {
        const int TableDirectoryOffset = 12;
        const int TableRecordSize = 16;
        const int RecordChecksumOffset = 4;
        const int RecordOffsetOffset = 8;
        const int RecordLengthOffset = 12;
        const int HeadCheckSumAdjustmentOffset = 8;
        const int HeadCreatedOffset = 20;
        const int HeadModifiedOffset = 28;
        const int HeadMinimumLength = 36;
        const uint ChecksumMagic = 0xB1B0AFBA;
        const long NoTimestamp = 0;

        public static void NormalizeFile(string path)
        {
            byte[] font = File.ReadAllBytes(path);
            Normalize(font);
            File.WriteAllBytes(path, font);
        }

        /// Rewrites <paramref name="font"/> in place.
        /// <exception cref="InvalidDataException">The data is not a well-formed sfnt with a `head` table.</exception>
        public static void Normalize(byte[] font)
        {
            if (font.Length < TableDirectoryOffset)
                throw new InvalidDataException("TTF is shorter than its offset table.");
            int tableCount = BinaryPrimitives.ReadUInt16BigEndian(font.AsSpan(4));
            int directoryEnd = TableDirectoryOffset + tableCount * TableRecordSize;
            if (font.Length < directoryEnd)
                throw new InvalidDataException("TTF table directory is truncated.");

            int headRecord = -1;
            for (int i = 0; i < tableCount; i++)
            {
                int record = TableDirectoryOffset + i * TableRecordSize;
                if (font.AsSpan(record, 4).SequenceEqual("head"u8))
                {
                    headRecord = record;
                    break;
                }
            }
            if (headRecord < 0)
                throw new InvalidDataException("TTF has no 'head' table.");

            int headOffset = ReadOffset(font, headRecord + RecordOffsetOffset);
            int headLength = ReadOffset(font, headRecord + RecordLengthOffset);
            if ((long)headOffset + headLength > font.Length)
                throw new InvalidDataException("TTF 'head' table extends past the end of the file.");
            if (headLength < HeadMinimumLength)
                throw new InvalidDataException("TTF 'head' table is too short.");

            BinaryPrimitives.WriteInt64BigEndian(font.AsSpan(headOffset + HeadCreatedOffset), NoTimestamp);
            BinaryPrimitives.WriteInt64BigEndian(font.AsSpan(headOffset + HeadModifiedOffset), NoTimestamp);
            BinaryPrimitives.WriteUInt32BigEndian(font.AsSpan(headOffset + HeadCheckSumAdjustmentOffset), 0);

            uint headChecksum = Checksum(font.AsSpan(headOffset, headLength));
            BinaryPrimitives.WriteUInt32BigEndian(font.AsSpan(headRecord + RecordChecksumOffset), headChecksum);

            uint adjustment = unchecked(ChecksumMagic - Checksum(font));
            BinaryPrimitives.WriteUInt32BigEndian(font.AsSpan(headOffset + HeadCheckSumAdjustmentOffset), adjustment);
        }

        static int ReadOffset(byte[] font, int at)
        {
            uint value = BinaryPrimitives.ReadUInt32BigEndian(font.AsSpan(at));
            if (value > int.MaxValue)
                throw new InvalidDataException("TTF table record is out of range.");
            return (int)value;
        }

        /// Sum of big-endian uint32 words, the final partial word zero-padded.
        static uint Checksum(ReadOnlySpan<byte> data)
        {
            uint sum = 0;
            int i = 0;
            for (; i + 4 <= data.Length; i += 4)
                sum = unchecked(sum + BinaryPrimitives.ReadUInt32BigEndian(data[i..]));
            if (i < data.Length)
            {
                Span<byte> tail = stackalloc byte[4];
                data[i..].CopyTo(tail);
                sum = unchecked(sum + BinaryPrimitives.ReadUInt32BigEndian(tail));
            }
            return sum;
        }
    }
}
