using System.Text;
using AssetConverter;

namespace InstallerTests
{
    /// <summary>Writes the byte and bit fields of SWF tags: little-endian integers, null-terminated strings and the packed RECT, MATRIX and CXFORM records.</summary>
    internal sealed class SwfBitWriter
    {
        private readonly List<byte> _bytes = [];
        private int _bitsUsed;

        public SwfBitWriter U8(int value)
        {
            Align();
            _bytes.Add((byte)value);
            return this;
        }

        public SwfBitWriter U16(int value) => Bytes(BitConverter.GetBytes((ushort)value));

        public SwfBitWriter S16(int value) => Bytes(BitConverter.GetBytes((short)value));

        public SwfBitWriter U32(uint value) => Bytes(BitConverter.GetBytes(value));

        /// <summary>A 16.16 fixed-point number.</summary>
        public SwfBitWriter Fixed(double value) => Bytes(BitConverter.GetBytes((int)Math.Round(value * 65536)));

        public SwfBitWriter Bytes(byte[] value)
        {
            Align();
            _bytes.AddRange(value);
            return this;
        }

        public SwfBitWriter String(string value) => Bytes([.. Encoding.UTF8.GetBytes(value), 0]);

        public SwfBitWriter Rgba(byte r, byte g, byte b, byte a) => Bytes([r, g, b, a]);

        public SwfBitWriter UB(long value, int count)
        {
            for (int i = count - 1; i >= 0; i--)
            {
                if (_bitsUsed == 0)
                    _bytes.Add(0);
                if (((value >> i) & 1) == 1)
                    _bytes[^1] |= (byte)(0x80 >> _bitsUsed);
                _bitsUsed = (_bitsUsed + 1) % 8;
            }
            return this;
        }

        public SwfBitWriter Flag(bool value) => UB(value ? 1 : 0, 1);

        public SwfBitWriter Align()
        {
            _bitsUsed = 0;
            return this;
        }

        public SwfBitWriter Rect(TwipRect rect)
        {
            long[] fields = [(long)rect.Xmin, (long)rect.Xmax, (long)rect.Ymin, (long)rect.Ymax];
            int size = SignedBits(fields);
            UB(size, 5);
            foreach (long field in fields)
                UB(field, size);
            return Align();
        }

        public SwfBitWriter Matrix(SwfMatrix matrix)
        {
            bool hasScale = matrix.A != 1 || matrix.D != 1;
            Flag(hasScale);
            if (hasScale)
                FixedPair(matrix.A, matrix.D);
            bool hasRotate = matrix.B != 0 || matrix.C != 0;
            Flag(hasRotate);
            if (hasRotate)
                FixedPair(matrix.B, matrix.C);
            long[] translate = [(long)matrix.Tx, (long)matrix.Ty];
            int size = SignedBits(translate);
            UB(size, 5);
            foreach (long field in translate)
                UB(field, size);
            return Align();
        }

        /// <summary>A CXFORMWITHALPHA; multipliers are fractions (1 = unchanged), add terms are channel values.</summary>
        public SwfBitWriter ColorTransform(AssetConverter.Swf.ColorTransform color)
        {
            long[] mult = [Unit(color.RMult), Unit(color.GMult), Unit(color.BMult), Unit(color.AMult)];
            long[] add = [(long)color.RAdd, (long)color.GAdd, (long)color.BAdd, (long)color.AAdd];
            bool hasMult = mult.Any(m => m != 256);
            bool hasAdd = add.Any(a => a != 0);
            Flag(hasAdd);
            Flag(hasMult);
            int size = SignedBits([.. hasMult ? mult : [], .. hasAdd ? add : [], 0]);
            UB(size, 4);
            if (hasMult)
                foreach (long field in mult)
                    UB(field, size);
            if (hasAdd)
                foreach (long field in add)
                    UB(field, size);
            return Align();
        }

        public byte[] ToArray() => [.. _bytes];

        private void FixedPair(double first, double second)
        {
            long[] fields = [(long)Math.Round(first * 65536), (long)Math.Round(second * 65536)];
            int size = SignedBits(fields);
            UB(size, 5);
            foreach (long field in fields)
                UB(field, size);
        }

        private static long Unit(double fraction) => (long)Math.Round(fraction * 256);

        private static int SignedBits(IEnumerable<long> fields)
        {
            int size = 1;
            foreach (long field in fields)
                while (field < -(1L << (size - 1)) || field >= (1L << (size - 1)))
                    size++;
            return size;
        }
    }
}
