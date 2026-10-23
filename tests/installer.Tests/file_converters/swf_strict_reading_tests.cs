using AssetConverter;
using AssetConverter.Swf;
using Xunit;

namespace InstallerTests
{
    // The SWF reader states what it cannot describe instead of guessing, so a changed
    // game file fails the conversion loudly.
    public sealed class SwfStrictReadingTests
    {
        private const int LengthOffset = 4;
        private static readonly SwfColor Black = new(0, 0, 0, 255);

        private static byte[] OneFrameMovie() => OneFrame(new SwfMovieBuilder());

        private static byte[] OneFrame(SwfMovieBuilder swf)
        {
            swf.Root.ShowFrame();
            return swf.Build();
        }

        private static byte[] WithStatedLength(byte[] swf, int length)
        {
            var patched = (byte[])swf.Clone();
            BitConverter.GetBytes((uint)length).CopyTo(patched, LengthOffset);
            return patched;
        }

        [Fact]
        public void Read_HeaderLengthThatDisagreesWithTheFile_Throws()
        {
            byte[] swf = OneFrameMovie();

            Assert.Throws<SwfFormatException>(() => SwfMovieReader.Read(WithStatedLength(swf, swf.Length + 1)));
        }

        [Fact]
        public void Read_BytesAfterTheEndTag_Throw()
        {
            byte[] swf = [.. OneFrameMovie(), 0x7F];

            var failure = Assert.Throws<SwfFormatException>(() => SwfMovieReader.Read(WithStatedLength(swf, swf.Length)));

            Assert.Contains("follow the End tag", failure.Message);
        }

        [Fact]
        public void Read_MovieWithoutAnEndTag_Throws()
        {
            byte[] swf = OneFrameMovie()[..^2];

            var failure = Assert.Throws<SwfFormatException>(() => SwfMovieReader.Read(WithStatedLength(swf, swf.Length)));

            Assert.Contains("no End tag", failure.Message);
        }

        [Fact]
        public void Read_LabelAfterTheLastFrame_Throws()
        {
            var swf = new SwfMovieBuilder();
            swf.Root.ShowFrame().Label("late");

            Assert.Throws<SwfFormatException>(() => SwfMovieReader.Read(swf.Build()));
        }

        [Fact]
        public void Read_SpriteThatHoldsOtherThanItsDeclaredFrames_Throws()
        {
            var swf = new SwfMovieBuilder();
            swf.Sprite(2, t => t.ShowFrame(), declaredFrameCount: 5);

            Assert.Throws<SwfFormatException>(() => SwfMovieReader.Read(OneFrame(swf)));
        }

        [Fact]
        public void Read_PlacementWithClipActions_Throws()
        {
            var swf = new SwfMovieBuilder();
            swf.Root.Tag(SwfTagCodes.PlaceObject2, [0x82, 0x01, 0x00, 0x01, 0x00]).ShowFrame();

            var failure = Assert.Throws<SwfFormatException>(() => SwfMovieReader.Read(swf.Build()));

            Assert.Contains("clip actions", failure.Message);
        }

        [Fact]
        public void Read_PlacementWithBytesItsFlagsDoNotDescribe_Throws()
        {
            var swf = new SwfMovieBuilder();
            swf.Root.Tag(SwfTagCodes.PlaceObject2, [0x02, 0x01, 0x00, 0x01, 0x00, 0xAA]).ShowFrame();

            Assert.Throws<SwfFormatException>(() => SwfMovieReader.Read(swf.Build()));
        }

        [Theory]
        [InlineData(0x04, true, false, false)]
        [InlineData(0x02, false, true, false)]
        [InlineData(0x01, false, false, true)]
        [InlineData(0x00, false, false, false)]
        public void Read_ShapeVersion4_KeepsItsFlags(int flags, bool windingRule, bool nonScaling, bool scaling)
        {
            var movie = OneFrameMovieWith(swf => swf.Shape(1, new TwipRect(0, 0, 20, 20), Black, version: 4, shapeFlags: (byte)flags));

            var shape = Assert.IsType<SwfShape>(movie.Characters[1]);
            Assert.Equal(new SwfShapeFlags(windingRule, nonScaling, scaling), shape.Flags);
        }

        [Fact]
        public void Read_ShapeBeforeVersion4_HasNoFlags()
        {
            var movie = OneFrameMovieWith(swf => swf.Shape(1, new TwipRect(0, 0, 20, 20), Black));

            Assert.Null(Assert.IsType<SwfShape>(movie.Characters[1]).Flags);
        }

        [Fact]
        public void Read_ShapeFlagsWithReservedBits_Throw()
        {
            var swf = new SwfMovieBuilder().Shape(1, new TwipRect(0, 0, 20, 20), Black, version: 4, shapeFlags: 0x08);
            swf.Root.ShowFrame();

            Assert.Throws<SwfFormatException>(() => swf.Read());
        }

        [Fact]
        public void Read_ShapeRecordSelectingAFillStyleItDoesNotHave_Throws()
        {
            var swf = new SwfMovieBuilder().PathShape(1, new TwipRect(0, 0, 400, 200), Black, null, 0, edges => edges.Style(0, 0, fillLeft: 2).Line(10, 0));
            swf.Root.ShowFrame();

            var failure = Assert.Throws<SwfFormatException>(() => swf.Read());

            Assert.Contains("fill style 2", failure.Message);
        }

        private static SwfMovie OneFrameMovieWith(Action<SwfMovieBuilder> define)
        {
            var swf = new SwfMovieBuilder();
            define(swf);
            swf.Root.ShowFrame();
            return swf.Read();
        }
    }
}
