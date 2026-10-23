using System.Buffers.Binary;
using System.Text;

namespace AssetConverter.ShaderTranslation.Dxbc
{
    public sealed class DxbcContainer
    {
        private const int HeaderSize = 32;
        private readonly Dictionary<string, ReadOnlyMemory<byte>> _chunks = new(StringComparer.Ordinal);

        public IReadOnlyDictionary<string, ReadOnlyMemory<byte>> Chunks => _chunks;

        public static DxbcContainer Parse(ReadOnlyMemory<byte> data)
        {
            var span = data.Span;
            if (span.Length < HeaderSize || !span[..4].SequenceEqual("DXBC"u8))
                throw new DxbcFormatException("Missing DXBC magic.");
            uint total = BinaryPrimitives.ReadUInt32LittleEndian(span[24..]);
            if (total > span.Length)
                throw new DxbcFormatException($"Container declares {total} bytes but only {span.Length} are available.");
            uint count = BinaryPrimitives.ReadUInt32LittleEndian(span[28..]);
            if (HeaderSize + (long)count * 4 > total)
                throw new DxbcFormatException($"Chunk table of {count} entries does not fit in the container.");

            var container = new DxbcContainer();
            for (int i = 0; i < count; i++)
            {
                uint offset = BinaryPrimitives.ReadUInt32LittleEndian(span[(HeaderSize + 4 * i)..]);
                if ((long)offset + 8 > total)
                    throw new DxbcFormatException($"Chunk {i} header at {offset} lies outside the container.");
                string fourCc = Encoding.ASCII.GetString(span.Slice((int)offset, 4));
                uint size = BinaryPrimitives.ReadUInt32LittleEndian(span[((int)offset + 4)..]);
                if ((long)offset + 8 + size > total)
                    throw new DxbcFormatException($"Chunk {fourCc} of {size} bytes overruns the container.");
                container._chunks[fourCc] = data.Slice((int)offset + 8, (int)size);
            }
            return container;
        }

        public ReadOnlyMemory<byte> RequireChunk(params string[] fourCcs)
        {
            foreach (var cc in fourCcs)
                if (_chunks.TryGetValue(cc, out var chunk)) return chunk;
            throw new DxbcFormatException($"Container has no {string.Join("/", fourCcs)} chunk.");
        }

        public IReadOnlyList<SignatureElement> ReadSignature(params string[] fourCcs)
        {
            foreach (var cc in fourCcs)
                if (_chunks.TryGetValue(cc, out var chunk))
                    return SignatureParser.Parse(chunk.Span, extended: cc.EndsWith('1'));
            return [];
        }
    }

    internal static class SignatureParser
    {
        public static IReadOnlyList<SignatureElement> Parse(ReadOnlySpan<byte> chunk, bool extended)
        {
            if (chunk.Length < 8)
                throw new DxbcFormatException("Signature chunk is truncated.");
            uint count = BinaryPrimitives.ReadUInt32LittleEndian(chunk);
            int stride = extended ? 32 : 24;
            if (8 + (long)count * stride > chunk.Length)
                throw new DxbcFormatException($"Signature declares {count} elements that do not fit in the chunk.");

            var elements = new List<SignatureElement>((int)count);
            for (int i = 0; i < count; i++)
            {
                var e = chunk[(8 + i * stride)..];
                int p = 0;
                uint stream = 0;
                if (extended)
                {
                    stream = BinaryPrimitives.ReadUInt32LittleEndian(e);
                    p = 4;
                }
                uint nameOffset = BinaryPrimitives.ReadUInt32LittleEndian(e[p..]);
                elements.Add(new SignatureElement(
                    ReadString(chunk, nameOffset),
                    BinaryPrimitives.ReadUInt32LittleEndian(e[(p + 4)..]),
                    BinaryPrimitives.ReadUInt32LittleEndian(e[(p + 8)..]),
                    BinaryPrimitives.ReadUInt32LittleEndian(e[(p + 12)..]),
                    BinaryPrimitives.ReadUInt32LittleEndian(e[(p + 16)..]),
                    e[p + 20], e[p + 21], stream));
            }
            return elements;
        }

        private static string ReadString(ReadOnlySpan<byte> chunk, uint offset)
        {
            if (offset >= chunk.Length)
                throw new DxbcFormatException($"Signature name offset {offset} is outside the chunk.");
            var tail = chunk[(int)offset..];
            int end = tail.IndexOf((byte)0);
            if (end < 0)
                throw new DxbcFormatException("Signature name is not terminated.");
            return Encoding.ASCII.GetString(tail[..end]);
        }
    }
}
