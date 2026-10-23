using System.Buffers.Binary;
using AssetConverter;
using Xunit;

namespace InstallerTests
{
    public class TtfNormalizerTests
    {
        const uint ChecksumMagic = 0xB1B0AFBA;
        const int HeadLength = 54;

        [Fact]
        public void FontsDifferingOnlyInTimestampsBecomeIdentical()
        {
            byte[] first = BuildFont(created: 3_873_415_951, modified: 3_873_415_951);
            byte[] second = BuildFont(created: 3_900_000_000, modified: 3_900_000_123);

            TtfNormalizer.Normalize(first);
            TtfNormalizer.Normalize(second);

            Assert.Equal(first, second);
        }

        [Fact]
        public void TimestampsAreZeroed()
        {
            byte[] font = BuildFont(created: 123, modified: 456);

            TtfNormalizer.Normalize(font);

            Assert.Equal(0L, BinaryPrimitives.ReadInt64BigEndian(font.AsSpan(HeadOffset(font) + 20)));
            Assert.Equal(0L, BinaryPrimitives.ReadInt64BigEndian(font.AsSpan(HeadOffset(font) + 28)));
        }

        [Fact]
        public void WholeFontChecksumEqualsMagicAndHeadRecordChecksumIsValid()
        {
            byte[] font = BuildFont(created: 123, modified: 456);

            TtfNormalizer.Normalize(font);

            Assert.Equal(ChecksumMagic, Sum(font));
            int head = HeadOffset(font);
            byte[] headTable = font.AsSpan(head, HeadLength).ToArray();
            BinaryPrimitives.WriteUInt32BigEndian(headTable.AsSpan(8), 0);
            Assert.Equal(Sum(headTable), BinaryPrimitives.ReadUInt32BigEndian(font.AsSpan(12 + 4)));
        }

        [Fact]
        public void NormalizingTwiceChangesNothing()
        {
            byte[] font = BuildFont(created: 123, modified: 456);
            TtfNormalizer.Normalize(font);
            byte[] once = (byte[])font.Clone();

            TtfNormalizer.Normalize(font);

            Assert.Equal(once, font);
        }

        [Fact]
        public void OtherTablesAreUntouched()
        {
            byte[] font = BuildFont(created: 123, modified: 456);
            byte[] before = (byte[])font.Clone();

            TtfNormalizer.Normalize(font);

            int namePayload = HeadOffset(font) + 56;
            Assert.Equal(before.AsSpan(namePayload).ToArray(), font.AsSpan(namePayload).ToArray());
            Assert.Equal(before.AsSpan(28, 16).ToArray(), font.AsSpan(28, 16).ToArray());
        }

        [Fact]
        public void FontWithoutHeadTableIsRejected()
        {
            byte[] font = BuildFont(created: 1, modified: 2);
            font[12] = (byte)'x';

            Assert.Throws<InvalidDataException>(() => TtfNormalizer.Normalize(font));
        }

        [Fact]
        public void TruncatedFontIsRejected()
        {
            byte[] font = BuildFont(created: 1, modified: 2);

            Assert.Throws<InvalidDataException>(() => TtfNormalizer.Normalize(font[..20]));
            Assert.Throws<InvalidDataException>(() => TtfNormalizer.Normalize(font[..(HeadOffset(font) + 10)]));
        }

        [Fact]
        public void NormalizeFileRewritesTheFileOnDisk()
        {
            string path = Path.Combine(Path.GetTempPath(), $"ttf-normalizer-{Guid.NewGuid():N}.ttf");
            try
            {
                File.WriteAllBytes(path, BuildFont(created: 123, modified: 456));

                TtfNormalizer.NormalizeFile(path);

                Assert.Equal(ChecksumMagic, Sum(File.ReadAllBytes(path)));
            }
            finally
            {
                File.Delete(path);
            }
        }

        static int HeadOffset(byte[] font) =>
            (int)BinaryPrimitives.ReadUInt32BigEndian(font.AsSpan(12 + 8));

        /// Two tables: `head` (record 0) then `name`, with an odd-length payload.
        static byte[] BuildFont(long created, long modified)
        {
            const int directoryEnd = 12 + 2 * 16;
            byte[] namePayload = [0xDE, 0xAD, 0xBE, 0xEF, 0x01, 0x02, 0x03];
            int nameOffset = directoryEnd + 56;
            byte[] font = new byte[nameOffset + 8];

            BinaryPrimitives.WriteUInt32BigEndian(font, 0x00010000);
            BinaryPrimitives.WriteUInt16BigEndian(font.AsSpan(4), 2);
            WriteRecord(font, 12, "head", checksum: 0x11111111, directoryEnd, HeadLength);
            WriteRecord(font, 28, "name", checksum: Sum(namePayload), nameOffset, namePayload.Length);

            BinaryPrimitives.WriteUInt32BigEndian(font.AsSpan(directoryEnd), 0x00010000);
            BinaryPrimitives.WriteUInt32BigEndian(font.AsSpan(directoryEnd + 8), 0x7777_7777);
            BinaryPrimitives.WriteUInt32BigEndian(font.AsSpan(directoryEnd + 12), 0x5F0F3CF5);
            BinaryPrimitives.WriteInt64BigEndian(font.AsSpan(directoryEnd + 20), created);
            BinaryPrimitives.WriteInt64BigEndian(font.AsSpan(directoryEnd + 28), modified);
            font[directoryEnd + 36] = 0x42;
            namePayload.CopyTo(font, nameOffset);
            return font;
        }

        static void WriteRecord(byte[] font, int at, string tag, uint checksum, int offset, int length)
        {
            System.Text.Encoding.ASCII.GetBytes(tag).CopyTo(font, at);
            BinaryPrimitives.WriteUInt32BigEndian(font.AsSpan(at + 4), checksum);
            BinaryPrimitives.WriteUInt32BigEndian(font.AsSpan(at + 8), (uint)offset);
            BinaryPrimitives.WriteUInt32BigEndian(font.AsSpan(at + 12), (uint)length);
        }

        static uint Sum(byte[] data)
        {
            byte[] padded = new byte[(data.Length + 3) / 4 * 4];
            data.CopyTo(padded, 0);
            uint sum = 0;
            for (int i = 0; i < padded.Length; i += 4)
                sum = unchecked(sum + BinaryPrimitives.ReadUInt32BigEndian(padded.AsSpan(i)));
            return sum;
        }
    }
}
