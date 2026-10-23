using System.Numerics;
using AssetConverter.Havok;
using Xunit;

namespace InstallerTests.Havok
{
    // Spline compressed encodings.
    public class HavokQuaternionCodecTests
    {
        private const float Tolerance = 1e-3f;

        [Fact]
        public void Decode_ThreeComp40IdentityBytes_ReturnsIdentity()
        {
            // a = b = c = 0 -> raw 2047 each; omitted component index 3 (w), positive:
            // v = 0x7FF | 0x7FF << 12 | 0x7FF << 24 | 3 << 36 = 0x37FF7FF7FF, little endian.
            var bytes = new HavokBytes([0xFF, 0xF7, 0x7F, 0xFF, 0x37]);

            var q = HavokQuaternionCodec.Decode(HavokRotationQuantization.ThreeComp40, bytes, 0);

            AssertQuaternion(Quaternion.Identity, q);
        }

        [Fact]
        public void Decode_ThreeComp24IdentityBytes_ReturnsIdentity()
        {
            // raw 63 per component (bias), omitted index 3 << 21: 0x6FDFBF little endian.
            var bytes = new HavokBytes([0xBF, 0xDF, 0x6F]);

            var q = HavokQuaternionCodec.Decode(HavokRotationQuantization.ThreeComp24, bytes, 0);

            AssertQuaternion(Quaternion.Identity, q);
        }

        [Fact]
        public void Decode_ThreeComp48IdentityWords_ReturnsIdentity()
        {
            // raw 0x3FFF per component; omitted index 3 splits over bit 15 of word 0 (low bit) and bit 15 of word 1 (high bit).
            var bytes = new HavokBytes([0xFF, 0xBF, 0xFF, 0xBF, 0xFF, 0x3F]);

            var q = HavokQuaternionCodec.Decode(HavokRotationQuantization.ThreeComp48, bytes, 0);

            AssertQuaternion(Quaternion.Identity, q);
        }

        [Fact]
        public void Decode_ThreeComp48OmittedFirstComponent_RecoversItFromTheOthers()
        {
            // (0.5, 0.5, 0.5, 0.5) with x omitted: y = z = w = 0.5 -> raw 0x3FFF + 0.5 / 0.000043161 = 27967 = 0x6D3F.
            var bytes = new HavokBytes([0x3F, 0x6D, 0x3F, 0x6D, 0x3F, 0x6D]);

            var q = HavokQuaternionCodec.Decode(HavokRotationQuantization.ThreeComp48, bytes, 0);

            AssertQuaternion(new Quaternion(0.5f, 0.5f, 0.5f, 0.5f), q);
        }

        [Fact]
        public void Decode_ThreeComp48SignBitSet_NegatesTheOmittedComponent()
        {
            // Same as above with bit 15 of the third word set: x becomes -0.5.
            var bytes = new HavokBytes([0x3F, 0x6D, 0x3F, 0x6D, 0x3F, 0xED]);

            var q = HavokQuaternionCodec.Decode(HavokRotationQuantization.ThreeComp48, bytes, 0);

            AssertQuaternion(new Quaternion(-0.5f, 0.5f, 0.5f, 0.5f), q);
        }

        [Fact]
        public void Decode_ThreeComp40ReadsAtTheGivenOffset()
        {
            var bytes = new HavokBytes([0xAA, 0xAA, 0xFF, 0xF7, 0x7F, 0xFF, 0x37]);

            var q = HavokQuaternionCodec.Decode(HavokRotationQuantization.ThreeComp40, bytes, 2);

            AssertQuaternion(Quaternion.Identity, q);
        }

        [Fact]
        public void Decode_Uncompressed_NormalizesTheStoredFloats()
        {
            var raw = new byte[16];
            BitConverter.GetBytes(2f).CopyTo(raw, 12);

            var q = HavokQuaternionCodec.Decode(HavokRotationQuantization.Uncompressed, new HavokBytes(raw), 0);

            AssertQuaternion(Quaternion.Identity, q);
        }

        [Theory]
        [InlineData(HavokRotationQuantization.Polar32)]
        [InlineData(HavokRotationQuantization.Straight16)]
        public void Decode_UnimplementedEncoding_ThrowsUnsupportedQuantization(HavokRotationQuantization encoding)
        {
            var bytes = new HavokBytes(new byte[16]);

            Assert.Throws<HavokUnsupportedQuantizationException>(() => HavokQuaternionCodec.Decode(encoding, bytes, 0));
        }

        [Fact]
        public void Decode_ThreeComp48CutOffBuffer_ThrowsFormatException()
        {
            var bytes = new HavokBytes([0xFF, 0xBF, 0xFF, 0xBF]);

            Assert.Throws<HavokFormatException>(() => HavokQuaternionCodec.Decode(HavokRotationQuantization.ThreeComp48, bytes, 0));
        }

        [Fact]
        public void Parse_UnknownValue_ThrowsFormatException()
        {
            Assert.Throws<HavokFormatException>(() => HavokQuaternionCodec.Parse(99));
        }

        [Theory]
        [InlineData(HavokRotationQuantization.ThreeComp40, 5, 1)]
        [InlineData(HavokRotationQuantization.ThreeComp48, 6, 2)]
        [InlineData(HavokRotationQuantization.ThreeComp24, 3, 1)]
        [InlineData(HavokRotationQuantization.Uncompressed, 16, 4)]
        public void ByteSizeAndAlignment_MatchTheDocumentedEncodingTable(HavokRotationQuantization encoding, int size, int alignment)
        {
            Assert.Equal(size, HavokQuaternionCodec.ByteSize(encoding));
            Assert.Equal(alignment, HavokQuaternionCodec.Alignment(encoding));
        }

        private static void AssertQuaternion(Quaternion expected, Quaternion actual)
        {
            Assert.InRange(MathF.Abs(expected.X - actual.X), 0f, Tolerance);
            Assert.InRange(MathF.Abs(expected.Y - actual.Y), 0f, Tolerance);
            Assert.InRange(MathF.Abs(expected.Z - actual.Z), 0f, Tolerance);
            Assert.InRange(MathF.Abs(expected.W - actual.W), 0f, Tolerance);
        }
    }

    public class HavokSplineCurveTests
    {
        private const float Tolerance = 1e-4f;

        [Fact]
        public void Evaluate_LinearSegment_InterpolatesBetweenControlPoints()
        {
            var curve = new HavokSplineCurve(degree: 1, knots: [0, 0, 4, 4], points: [0f, 10f], dimensions: 1);
            Span<float> result = stackalloc float[1];

            curve.Evaluate(1f, result);

            Assert.Equal(2.5f, result[0], Tolerance);
        }

        [Fact]
        public void Evaluate_BeyondTheLastKnot_ReturnsTheLastControlPoint()
        {
            var curve = new HavokSplineCurve(1, [0, 0, 4, 4], [0f, 10f], 1);
            Span<float> result = stackalloc float[1];

            curve.Evaluate(99f, result);

            Assert.Equal(10f, result[0], Tolerance);
        }

        [Fact]
        public void Evaluate_CubicBezier_MatchesTheClosedFormBernsteinPolynomial()
        {
            // Control points 0, 0, 3, 3 over a clamped knot vector: B(t) = 9t^2 - 6t^3, B(0.5) = 1.5.
            var curve = new HavokSplineCurve(3, [0, 0, 0, 0, 10, 10, 10, 10], [0f, 0f, 3f, 3f], 1);
            Span<float> result = stackalloc float[1];

            curve.Evaluate(5f, result);

            Assert.Equal(1.5f, result[0], Tolerance);
        }

        [Theory]
        [InlineData(0f)]
        [InlineData(0.7f)]
        [InlineData(1.5f)]
        [InlineData(2.2f)]
        [InlineData(3f)]
        public void Evaluate_QuadraticWithGrevilleControlPoints_ReproducesTheParameter(float u)
        {
            // Control points at the Greville abscissae (0, 0.5, 1.5, 2.5, 3) make a B-spline reproduce f(u) = u.
            var curve = new HavokSplineCurve(2, [0, 0, 0, 1, 2, 3, 3, 3], [0f, 0.5f, 1.5f, 2.5f, 3f], 1);
            Span<float> result = stackalloc float[1];

            curve.Evaluate(u, result);

            Assert.Equal(u, result[0], Tolerance);
        }

        [Fact]
        public void Evaluate_TwoDimensionalPoints_InterpolatesEachDimension()
        {
            var curve = new HavokSplineCurve(1, [0, 0, 2, 2], [0f, 100f, 10f, 0f], 2);
            Span<float> result = stackalloc float[2];

            curve.Evaluate(1f, result);

            Assert.Equal(5f, result[0], Tolerance);
            Assert.Equal(50f, result[1], Tolerance);
        }

        [Fact]
        public void Constructor_TooFewControlPointsForTheDegree_Throws()
        {
            Assert.Throws<HavokFormatException>(() => new HavokSplineCurve(3, [0, 0, 0, 0, 1, 1, 1], [0f, 1f, 2f], 1));
        }

        [Fact]
        public void Constructor_WrongKnotCount_Throws()
        {
            Assert.Throws<HavokFormatException>(() => new HavokSplineCurve(1, [0, 0, 1], [0f, 1f], 1));
        }

        [Fact]
        public void Constructor_DecreasingKnots_Throws()
        {
            Assert.Throws<HavokFormatException>(() => new HavokSplineCurve(1, [0, 5, 3, 9], [0f, 1f], 1));
        }
    }
}
