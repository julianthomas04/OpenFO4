using System.Buffers.Binary;

namespace AssetConverter.ShaderTranslation.Archive
{
    /// <summary>
    /// Parses the shader package: a sequence of groups, each a 20-byte header followed by entries
    /// (marker 0x11223344, DXBC size, technique descriptor, slot byte table, DXBC container).
    /// </summary>
    public static class FxpParser
    {
        public const uint EntryMarker = 0x11223344;
        private const int GroupHeaderSize = 20;
        private const int EntryFixedSize = 12;
        private const int MaxSlotTableSize = 256;
        private const int DxbcHeaderSize = 32;

        private static ReadOnlySpan<byte> DxbcMagic => "DXBC"u8;

        public static FxpFile Parse(ReadOnlyMemory<byte> data)
        {
            var groups = new List<FxpGroupHeader>();
            var entries = new List<FxpEntry>();
            int pos = 0;
            while (pos < data.Length)
            {
                if (ReadU32(data.Span, pos) == EntryMarker)
                {
                    if (groups.Count == 0)
                        throw new FxpFormatException(pos, "Entry found before any group header.");
                    pos = ParseEntry(data, pos, groups.Count - 1, entries);
                }
                else
                {
                    groups.Add(ParseGroupHeader(data.Span, pos));
                    pos += GroupHeaderSize;
                }
            }
            return new FxpFile(groups, entries);
        }

        private static FxpGroupHeader ParseGroupHeader(ReadOnlySpan<byte> data, int pos)
        {
            if (pos + GroupHeaderSize > data.Length)
                throw new FxpFormatException(pos, "Truncated group header.");
            return new FxpGroupHeader(
                ReadU32(data, pos), ReadU32(data, pos + 4), ReadU32(data, pos + 8), ReadU32(data, pos + 12), ReadU32(data, pos + 16));
        }

        private static int ParseEntry(ReadOnlyMemory<byte> data, int pos, int groupIndex, List<FxpEntry> entries)
        {
            var span = data.Span;
            if (pos + EntryFixedSize > span.Length)
                throw new FxpFormatException(pos, "Truncated entry header.");
            uint size = ReadU32(span, pos + 4);
            uint descriptor = ReadU32(span, pos + 8);
            int tableStart = pos + EntryFixedSize;
            int dxbcStart = FindDxbc(span, tableStart, size);
            if (dxbcStart < 0)
                throw new FxpFormatException(pos, $"No DXBC container of {size} bytes within {MaxSlotTableSize} bytes of the entry header.");
            if ((long)dxbcStart + size > span.Length)
                throw new FxpFormatException(pos, $"DXBC container of {size} bytes runs past the end of the file.");

            entries.Add(new FxpEntry(
                entries.Count, groupIndex, descriptor,
                data.Slice(tableStart, dxbcStart - tableStart),
                data.Slice(dxbcStart, (int)size)));
            return dxbcStart + (int)size;
        }

        private static int FindDxbc(ReadOnlySpan<byte> data, int from, uint expectedSize)
        {
            int last = Math.Min(from + MaxSlotTableSize, data.Length - DxbcHeaderSize);
            for (int p = from; p <= last; p++)
            {
                if (!data.Slice(p, 4).SequenceEqual(DxbcMagic)) continue;
                if (ReadU32(data, p + 20) == 1 && ReadU32(data, p + 24) == expectedSize)
                    return p;
            }
            return -1;
        }

        private static uint ReadU32(ReadOnlySpan<byte> data, int pos)
        {
            if (pos + 4 > data.Length)
                throw new FxpFormatException(pos, "Truncated 32-bit value.");
            return BinaryPrimitives.ReadUInt32LittleEndian(data.Slice(pos, 4));
        }
    }
}
