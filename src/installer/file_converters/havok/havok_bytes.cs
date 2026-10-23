using System.Buffers.Binary;
using System.Text;

namespace AssetConverter.Havok
{
    /// <summary>Bounds-checked little-endian view over an untrusted Havok buffer.</summary>
    public sealed class HavokBytes
    {
        private readonly byte[] _data;

        public HavokBytes(byte[] data)
        {
            _data = data ?? throw new ArgumentNullException(nameof(data));
        }

        public int Length => _data.Length;

        public void Require(long offset, long length, string what)
        {
            if (offset < 0 || length < 0 || offset + length > _data.Length)
                throw new HavokFormatException($"{what}: range [{offset}, {offset + length}) exceeds buffer of {_data.Length} bytes");
        }

        public ReadOnlySpan<byte> Slice(long offset, long length, string what)
        {
            Require(offset, length, what);
            return _data.AsSpan((int)offset, (int)length);
        }

        public byte U8(long offset) { Require(offset, 1, "u8"); return _data[offset]; }
        public ushort U16(long offset) { Require(offset, 2, "u16"); return BinaryPrimitives.ReadUInt16LittleEndian(_data.AsSpan((int)offset)); }
        public short I16(long offset) { Require(offset, 2, "i16"); return BinaryPrimitives.ReadInt16LittleEndian(_data.AsSpan((int)offset)); }
        public uint U32(long offset) { Require(offset, 4, "u32"); return BinaryPrimitives.ReadUInt32LittleEndian(_data.AsSpan((int)offset)); }
        public int I32(long offset) { Require(offset, 4, "i32"); return BinaryPrimitives.ReadInt32LittleEndian(_data.AsSpan((int)offset)); }
        public ulong U64(long offset) { Require(offset, 8, "u64"); return BinaryPrimitives.ReadUInt64LittleEndian(_data.AsSpan((int)offset)); }
        public float F32(long offset) { Require(offset, 4, "f32"); return BinaryPrimitives.ReadSingleLittleEndian(_data.AsSpan((int)offset)); }

        public string CString(long offset, int maxLength, string what)
        {
            Require(offset, 1, what);
            int limit = (int)Math.Min(_data.Length, offset + maxLength);
            int end = Array.IndexOf(_data, (byte)0, (int)offset, limit - (int)offset);
            if (end < 0)
                throw new HavokFormatException($"{what}: unterminated string at {offset}");
            return Encoding.UTF8.GetString(_data, (int)offset, end - (int)offset);
        }

        public byte[] Copy(long offset, long length, string what) => Slice(offset, length, what).ToArray();
    }
}
