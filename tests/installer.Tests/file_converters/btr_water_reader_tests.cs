using AssetConverter;
using Xunit;

namespace InstallerTests
{
    // Terrain LOD water layout.
    public sealed class BtrWaterReaderTests
    {
        const string Entry = "Meshes\\Terrain\\Fixture\\Fixture.4.-12.8.BTR";

        static IReadOnlyList<LodWaterQuad> Read(BtrBuilder builder, int blockX = -12, int blockY = 8) =>
            BtrWaterReader.Read(builder.Build(), Entry, blockX, blockY);

        [Fact]
        public void Read_QuadsOfOneShape_AreOffsetFromBlockOrigin()
        {
            var quads = Read(new BtrBuilder().Shape(new FixtureQuad(0, 0, 450), new FixtureQuad(3, 1, 1300)));

            Assert.Equal([new LodWaterQuad(-12, 8, 450), new LodWaterQuad(-9, 9, 1300)], quads);
        }

        [Fact]
        public void Read_SeveralShapes_ConcatenatesQuadsInShapeOrder()
        {
            var quads = Read(new BtrBuilder().Shape(new FixtureQuad(0, 0, 450)).Shape(new FixtureQuad(1, 0, 458)));

            Assert.Equal([new LodWaterQuad(-12, 8, 450), new LodWaterQuad(-11, 8, 458)], quads);
        }

        [Theory]
        [InlineData(-37.5)]
        [InlineData(0.0)]
        [InlineData(113.25)]
        public void Read_Height_IsTheExactDecodedHalfTimesScale(double height)
        {
            var quad = Assert.Single(Read(new BtrBuilder().Shape(new FixtureQuad(0, 0, height))));

            Assert.Equal(height, quad.Height);
        }

        [Fact]
        public void Read_WithoutWaterNode_ReturnsNoQuads()
        {
            Assert.Empty(Read(new BtrBuilder { IncludeWaterNode = false }));
        }

        [Theory]
        [InlineData("descriptor")]
        [InlineData("shapeType")]
        [InlineData("waterTransform")]
        [InlineData("topology")]
        [InlineData("flatness")]
        public void Read_UnexpectedLayout_ThrowsNamingTheFile(string defect)
        {
            var builder = new BtrBuilder().Shape(new FixtureQuad(0, 0, 450));
            switch (defect)
            {
                case "descriptor": builder.VertexDescriptor = 0x100000000003; break;
                case "shapeType": builder.ShapeType = "BSTriShape"; break;
                case "waterTransform": builder.WaterNodeScale = 2f; break;
                case "topology": builder.TriangleIndexShift = 1; break;
                case "flatness": builder.VertexZSkew = 4; break;
            }

            var e = Assert.Throws<BtrFormatException>(() => Read(builder));
            Assert.Contains(Entry, e.Message);
        }

        [Fact]
        public void Read_TruncatedFile_Throws()
        {
            var bytes = new BtrBuilder().Shape(new FixtureQuad(0, 0, 450)).Build();

            Assert.Throws<BtrFormatException>(() => BtrWaterReader.Read(bytes[..(bytes.Length - 10)], Entry, 0, 0));
        }

        [Fact]
        public void Read_NotANif_Throws()
        {
            Assert.Throws<BtrFormatException>(() => BtrWaterReader.Read([1, 2, 3], Entry, 0, 0));
        }
    }
}
