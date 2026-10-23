using System.Numerics;

namespace AssetConverter.Havok
{
    public enum HavokRotationQuantization
    {
        Polar32 = 0,
        ThreeComp40 = 1,
        ThreeComp48 = 2,
        ThreeComp24 = 3,
        Straight16 = 4,
        Uncompressed = 5,
    }

    /// <summary>Decoders for the hkaSplineCompressedAnimation quaternion encodings (smallest-three family and raw floats).</summary>
    public static class HavokQuaternionCodec
    {
        private const float Scale48 = 0.000043161f;
        private const float Scale40 = 0.000345436f;
        private const float Scale24 = 0.011135f;

        public static HavokRotationQuantization Parse(int value)
        {
            if (!Enum.IsDefined(typeof(HavokRotationQuantization), value))
                throw new HavokFormatException($"Unknown rotation quantization {value}");
            return (HavokRotationQuantization)value;
        }

        public static int ByteSize(HavokRotationQuantization q) => q switch
        {
            HavokRotationQuantization.Polar32 => 4,
            HavokRotationQuantization.ThreeComp40 => 5,
            HavokRotationQuantization.ThreeComp48 => 6,
            HavokRotationQuantization.ThreeComp24 => 3,
            HavokRotationQuantization.Straight16 => 2,
            HavokRotationQuantization.Uncompressed => 16,
            _ => throw new ArgumentOutOfRangeException(nameof(q)),
        };

        public static int Alignment(HavokRotationQuantization q) => q switch
        {
            HavokRotationQuantization.Polar32 => 4,
            HavokRotationQuantization.ThreeComp40 => 1,
            HavokRotationQuantization.ThreeComp48 => 2,
            HavokRotationQuantization.ThreeComp24 => 1,
            HavokRotationQuantization.Straight16 => 2,
            HavokRotationQuantization.Uncompressed => 4,
            _ => throw new ArgumentOutOfRangeException(nameof(q)),
        };

        public static void RequireSupported(HavokRotationQuantization q)
        {
            if (q is HavokRotationQuantization.Polar32 or HavokRotationQuantization.Straight16)
                throw new HavokUnsupportedQuantizationException($"Rotation quantization {q} is not implemented");
        }

        public static Quaternion Decode(HavokRotationQuantization q, HavokBytes bytes, int offset)
        {
            RequireSupported(q);
            switch (q)
            {
                case HavokRotationQuantization.ThreeComp48:
                    {
                        ushort s0 = bytes.U16(offset), s1 = bytes.U16(offset + 2), s2 = bytes.U16(offset + 4);
                        int missing = ((s1 >> 14) & 2) | (s0 >> 15);
                        return SmallestThree(
                            Component(s0 & 0x7FFF, 0x3FFF, Scale48), Component(s1 & 0x7FFF, 0x3FFF, Scale48),
                            Component(s2 & 0x7FFF, 0x3FFF, Scale48), missing, (s2 >> 15) != 0);
                    }
                case HavokRotationQuantization.ThreeComp40:
                    {
                        ulong v = 0;
                        for (int i = 4; i >= 0; i--) v = (v << 8) | bytes.U8(offset + i);
                        return SmallestThree(
                            Component((int)(v & 0xFFF), 2047, Scale40), Component((int)((v >> 12) & 0xFFF), 2047, Scale40),
                            Component((int)((v >> 24) & 0xFFF), 2047, Scale40), (int)((v >> 36) & 3), ((v >> 38) & 1) != 0);
                    }
                case HavokRotationQuantization.ThreeComp24:
                    {
                        uint v = (uint)(bytes.U8(offset) | bytes.U8(offset + 1) << 8 | bytes.U8(offset + 2) << 16);
                        return SmallestThree(
                            Component((int)(v & 0x7F), 63, Scale24), Component((int)((v >> 7) & 0x7F), 63, Scale24),
                            Component((int)((v >> 14) & 0x7F), 63, Scale24), (int)((v >> 21) & 3), ((v >> 23) & 1) != 0);
                    }
                case HavokRotationQuantization.Uncompressed:
                    return Quaternion.Normalize(new Quaternion(bytes.F32(offset), bytes.F32(offset + 4), bytes.F32(offset + 8), bytes.F32(offset + 12)));
                default:
                    throw new ArgumentOutOfRangeException(nameof(q));
            }
        }

        private static float Component(int raw, int bias, float scale) => (raw - bias) * scale;

        private static Quaternion SmallestThree(float a, float b, float c, int missing, bool negativeMissing)
        {
            float restSquared = 1f - a * a - b * b - c * c;
            float recovered = MathF.Sqrt(MathF.Max(restSquared, 0f));
            if (negativeMissing) recovered = -recovered;

            Span<float> q = stackalloc float[4];
            Span<float> given = stackalloc float[] { a, b, c };
            int next = 0;
            for (int i = 0; i < 4; i++)
                q[i] = i == missing ? recovered : given[next++];
            return Quaternion.Normalize(new Quaternion(q[0], q[1], q[2], q[3]));
        }
    }
}
