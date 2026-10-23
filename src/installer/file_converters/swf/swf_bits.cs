using System.Text;

namespace AssetConverter.Swf
{
    /// <summary>
    /// Little-endian byte and big-endian bit-field reader over one SWF tag or the movie body, as the SWF file format lays its values out.
    /// Every read past the end throws <see cref="SwfFormatException"/> naming what was being read.
    /// </summary>
    internal sealed class SwfBits
    {
        private readonly byte[] _data;
        private readonly int _end;
        private int _bitPosition;

        public SwfBits(byte[] data)
            : this(data, 0, data.Length)
        {
        }

        public SwfBits(byte[] data, int start, int length)
        {
            if (start < 0 || length < 0 || start + length > data.Length)
                throw new SwfFormatException($"A span of {length} bytes at {start} lies outside the {data.Length} bytes read.");
            _data = data;
            Position = start;
            _end = start + length;
        }

        public int Position { get; private set; }

        public int Remaining => _end - Position;

        public bool AtEnd => Position >= _end;

        public byte[] Data => _data;

        public void Skip(int count)
        {
            Align();
            Require(count, "skipped bytes");
            Position += count;
        }

        public byte U8()
        {
            Align();
            Require(1, "a byte");
            return _data[Position++];
        }

        public ushort U16()
        {
            Align();
            Require(2, "a 16-bit value");
            ushort value = BitConverter.ToUInt16(_data, Position);
            Position += 2;
            return value;
        }

        public short S16() => unchecked((short)U16());

        public uint U32()
        {
            Align();
            Require(4, "a 32-bit value");
            uint value = BitConverter.ToUInt32(_data, Position);
            Position += 4;
            return value;
        }

        public float Float32()
        {
            Align();
            Require(4, "a float");
            float value = BitConverter.ToSingle(_data, Position);
            Position += 4;
            return value;
        }

        /// <summary>A signed 16.16 fixed-point value.</summary>
        public double Fixed() => unchecked((int)U32()) / 65536.0;

        /// <summary>A signed 8.8 fixed-point value.</summary>
        public double Fixed8() => S16() / 256.0;

        /// <summary>The SWF variable-length unsigned integer: 7 bits per byte, at most 5 bytes.</summary>
        public uint EncodedU32()
        {
            uint result = 0;
            for (int i = 0; i < 5; i++)
            {
                byte b = U8();
                result |= (uint)(b & 0x7F) << (7 * i);
                if ((b & 0x80) == 0)
                    return result;
            }
            throw new SwfFormatException("An encoded 32-bit value runs past five bytes.");
        }

        /// <summary>A null-terminated UTF-8 string.</summary>
        public string String()
        {
            Align();
            int terminator = Array.IndexOf(_data, (byte)0, Position, _end - Position);
            if (terminator < 0)
                throw new SwfFormatException($"A string at {Position} has no terminating zero byte.");
            string value = Encoding.UTF8.GetString(_data, Position, terminator - Position);
            Position = terminator + 1;
            return value;
        }

        public byte[] Bytes(int count)
        {
            Align();
            Require(count, $"{count} bytes");
            byte[] value = _data.AsSpan(Position, count).ToArray();
            Position += count;
            return value;
        }

        public byte[] RestOfSpan() => Bytes(Remaining);

        public uint UB(int bits)
        {
            if (bits is < 0 or > 32)
                throw new SwfFormatException($"A bit field of {bits} bits is not readable.");
            uint value = 0;
            for (int i = 0; i < bits; i++)
            {
                if (_bitPosition == 0)
                    Require(1, "a bit field");
                int bit = (_data[Position] >> (7 - _bitPosition)) & 1;
                value = (value << 1) | (uint)bit;
                _bitPosition++;
                if (_bitPosition == 8)
                {
                    _bitPosition = 0;
                    Position++;
                }
            }
            return value;
        }

        public int SB(int bits)
        {
            uint raw = UB(bits);
            if (bits == 0)
                return 0;
            if (bits < 32 && (raw & (1u << (bits - 1))) != 0)
                return unchecked((int)(raw | (uint.MaxValue << bits)));
            return unchecked((int)raw);
        }

        /// <summary>A signed 16.16 fixed-point bit field.</summary>
        public double FB(int bits) => SB(bits) / 65536.0;

        public bool Flag() => UB(1) == 1;

        /// <summary>Moves to the next byte boundary; bit fields always end there before a byte-aligned value is read.</summary>
        public void Align()
        {
            if (_bitPosition == 0)
                return;
            _bitPosition = 0;
            Position++;
        }

        public TwipRect Rect()
        {
            Align();
            int bits = (int)UB(5);
            int xMin = SB(bits);
            int xMax = SB(bits);
            int yMin = SB(bits);
            int yMax = SB(bits);
            Align();
            return new TwipRect(xMin, yMin, xMax, yMax);
        }

        public SwfMatrix Matrix()
        {
            Align();
            double a = 1, d = 1, b = 0, c = 0;
            if (Flag())
            {
                int scaleBits = (int)UB(5);
                a = FB(scaleBits);
                d = FB(scaleBits);
            }
            if (Flag())
            {
                int rotateBits = (int)UB(5);
                b = FB(rotateBits);
                c = FB(rotateBits);
            }
            int translateBits = (int)UB(5);
            int tx = SB(translateBits);
            int ty = SB(translateBits);
            Align();
            return new SwfMatrix(a, b, c, d, tx, ty);
        }

        /// <summary>A colour transform; <paramref name="withAlpha"/> selects CXFORMWITHALPHA over CXFORM (which leaves alpha unchanged).</summary>
        public ColorTransform ColorTransform(bool withAlpha)
        {
            Align();
            bool hasAdd = Flag();
            bool hasMult = Flag();
            int bits = (int)UB(4);
            int channels = withAlpha ? 4 : 3;
            int[] mult = [256, 256, 256, 256];
            int[] add = [0, 0, 0, 0];
            if (hasMult)
                for (int i = 0; i < channels; i++)
                    mult[i] = SB(bits);
            if (hasAdd)
                for (int i = 0; i < channels; i++)
                    add[i] = SB(bits);
            Align();
            return Swf.ColorTransform.FromTerms(mult, add);
        }

        public SwfColor Rgb() => new(U8(), U8(), U8(), 255);

        public SwfColor Rgba() => new(U8(), U8(), U8(), U8());

        public SwfColor Argb()
        {
            byte a = U8();
            return new SwfColor(U8(), U8(), U8(), a);
        }

        private void Require(int count, string what)
        {
            if (count < 0 || Position + count > _end)
                throw new SwfFormatException($"The data ends while reading {what} at {Position} ({_end - Position} bytes left).");
        }
    }
}
