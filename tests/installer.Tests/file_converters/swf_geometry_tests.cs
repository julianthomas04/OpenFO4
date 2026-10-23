using AssetConverter;
using Xunit;

namespace InstallerTests
{
    public sealed class SwfGeometryTests
    {
        private static readonly SwfMatrix QuarterTurn = new(A: 0, B: 1, C: -1, D: 0, Tx: 100, Ty: 200);

        private static void AssertPoint((double X, double Y) expected, (double X, double Y) actual)
        {
            Assert.Equal(expected.X, actual.X, 9);
            Assert.Equal(expected.Y, actual.Y, 9);
        }

        [Fact]
        public void Compose_AppliesTheChildFirstAndTheParentSecond()
        {
            (SwfMatrix Parent, SwfMatrix Child)[] pairs =
            [
                (new SwfMatrix(1.5, 0.5, 0.25, 2, 30, -40), new SwfMatrix(0.5, -0.75, 1.25, 1, -7, 11)),
                (QuarterTurn, new SwfMatrix(2, 0.1, -0.2, 3, 5, 9)),
                (new SwfMatrix(-1, 0.3, 0.6, 1, 0, 0), new SwfMatrix(1, 1, 1, 1, 2, 3)),
            ];

            foreach ((SwfMatrix parent, SwfMatrix child) in pairs)
                foreach ((double x, double y) in new[] { (0.0, 0.0), (10.0, 0.0), (0.0, 10.0), (-7.0, 13.0) })
                {
                    (double childX, double childY) = child.Apply(x, y);
                    AssertPoint(parent.Apply(childX, childY), parent.Compose(child).Apply(x, y));
                }
        }

        [Fact]
        public void Compose_IsNotCommutative()
        {
            var shift = new SwfMatrix(1, 0, 0, 1, 5, 0);

            AssertPoint((100, 205), QuarterTurn.Compose(shift).Apply(0, 0));
            AssertPoint((105, 200), shift.Compose(QuarterTurn).Apply(0, 0));
        }

        [Fact]
        public void Compose_RotatedParentCarriesTheChildsVerticalOffsetIntoX()
        {
            var shiftDown = new SwfMatrix(1, 0, 0, 1, 0, 5);

            var composed = QuarterTurn.Compose(shiftDown);

            Assert.Equal(95, composed.Tx);
            Assert.Equal(200, composed.Ty);
        }

        [Fact]
        public void Transform_ARotatedBox_IsTheBoundingBoxOfItsRotatedCorners()
        {
            var box = new TwipRect(0, 0, 10, 20);

            var rotated = box.Transform(new SwfMatrix(0, 1, -1, 0, 0, 0));

            Assert.Equal(new TwipRect(-20, 0, 0, 10), rotated);
        }

        [Fact]
        public void Transform_ASkewedBox_CoversAllFourSkewedCorners()
        {
            var box = new TwipRect(0, 0, 10, 10);

            var skewed = box.Transform(new SwfMatrix(1, 0, 1, 1, 0, 0));

            Assert.Equal(new TwipRect(0, 0, 20, 10), skewed);
        }

        [Theory]
        [InlineData(1.0, 0, 0, 1.0, true)]
        [InlineData(1.0000002, 0, 0, 0.9999998, true)]
        [InlineData(1.01, 0, 0, 1.0, false)]
        [InlineData(1.0, 0.001, 0, 1.0, false)]
        [InlineData(1.0, 0, -0.001, 1.0, false)]
        public void IsTranslationOnly_AllowsFixedPointRoundingButNotARealScaleOrRotation(double a, double b, double c, double d, bool expected)
        {
            Assert.Equal(expected, new SwfMatrix(a, b, c, d, 12, 34).IsTranslationOnly);
        }
    }
}
