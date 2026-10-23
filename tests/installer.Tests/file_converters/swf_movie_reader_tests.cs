using System.Text;
using AssetConverter;
using AssetConverter.Swf;
using Xunit;

namespace InstallerTests
{
    public sealed class SwfMovieReaderTests
    {
        private static SwfMovie Read(Action<SwfMovieBuilder> define, bool compressed = false)
        {
            var swf = new SwfMovieBuilder { Compressed = compressed, Version = 15, FrameRate = 24.5, Stage = new TwipRect(0, 0, 25600, 14400) };
            define(swf);
            swf.Root.ShowFrame();
            return swf.Read();
        }

        [Theory]
        [InlineData(false)]
        [InlineData(true)]
        public void Read_Header_GivesVersionStageAndFrameRate(bool compressed)
        {
            var movie = Read(_ => { }, compressed);

            Assert.Equal(15, movie.Version);
            Assert.Equal(new TwipRect(0, 0, 25600, 14400), movie.Stage);
            Assert.Equal(24.5, movie.FrameRate);
            Assert.Single(movie.Root.Frames);
        }

        [Theory]
        [InlineData("GFX")]
        [InlineData("FWS")]
        public void Read_ScaleformAndPlainSignatures_ReadAlike(string signature)
        {
            byte[] bytes = new SwfMovieBuilder().Build();
            Encoding.ASCII.GetBytes(signature).CopyTo(bytes, 0);

            Assert.Equal(new SwfMovieBuilder().Read().Stage, SwfMovieReader.Read(bytes).Stage);
        }

        [Fact]
        public void Read_UnknownSignature_Throws()
        {
            byte[] bytes = new SwfMovieBuilder().Build();
            Encoding.ASCII.GetBytes("ZWS").CopyTo(bytes, 0);

            Assert.Throws<SwfFormatException>(() => SwfMovieReader.Read(bytes));
        }

        [Fact]
        public void Read_CompressedBodyThatDoesNotInflate_Throws()
        {
            byte[] bytes = [.. Encoding.ASCII.GetBytes("CWS"), 10, 100, 0, 0, 0, 1, 2, 3, 4];

            Assert.Throws<SwfFormatException>(() => SwfMovieReader.Read(bytes));
        }

        [Fact]
        public void Read_TruncatedFile_Throws()
        {
            byte[] bytes = new SwfMovieBuilder().Build();

            Assert.Throws<SwfFormatException>(() => SwfMovieReader.Read(bytes[..10]));
        }

        [Fact]
        public void Read_TagLongerThanTheFile_Throws()
        {
            var swf = new SwfMovieBuilder().Tag(SwfTagCodes.SymbolClass, new byte[80]);
            byte[] bytes = swf.Build();

            Assert.Throws<SwfFormatException>(() => SwfMovieReader.Read(bytes[..^40]));
        }

        [Fact]
        public void Read_CharacterDefinedTwice_Throws()
        {
            Assert.Throws<SwfFormatException>(() => Read(swf => swf.Shape(1, new TwipRect(0, 0, 10, 10), new SwfColor(0, 0, 0, 255)).Sprite(1, t => t.ShowFrame())));
        }

        [Theory]
        [InlineData(1)]
        [InlineData(2)]
        [InlineData(3)]
        [InlineData(4)]
        public void Read_Shape_GivesItsBoundsAndSolidFill(int version)
        {
            var movie = Read(swf => swf.Shape(3, new TwipRect(-20, -40, 60, 80), new SwfColor(10, 20, 30, 128), version, new TwipRect(0, 0, 40, 40)));

            var shape = Assert.IsType<SwfShape>(movie.Characters[3]);
            Assert.Equal(version, shape.Version);
            Assert.Equal(new TwipRect(-20, -40, 60, 80), shape.Bounds);
            Assert.Equal(version == 4 ? new TwipRect(0, 0, 40, 40) : null, shape.EdgeBounds);
            var fill = Assert.Single(shape.Fills);
            Assert.Equal(SwfFillType.Solid, fill.Type);
            Assert.Equal(new SwfColor(10, 20, 30, version >= 3 ? (byte)128 : (byte)255), fill.Color);
        }

        private static readonly SwfColor White = new(255, 255, 255, 255);

        private static SwfShape ReadPathShape(SwfColor? stroke, Action<SwfShapeRecordWriter> records) =>
            Assert.IsType<SwfShape>(Read(swf => swf.PathShape(1, new TwipRect(0, 0, 400, 200), White, stroke, 20, records)).Characters[1]);

        [Fact]
        public void Read_ShapeOutline_GivesEachRunOfEdgesWithItsStyles()
        {
            var shape = ReadPathShape(White, edges => edges
                .Style(0, 0, fillLeft: 1, line: 1).Line(400, 0).Line(0, 200).Line(-400, 0).Line(0, -200)
                .Style(100, 50, fillLeft: 0, fillRight: 1, line: 0).Line(20, 0));

            Assert.Equal(2, shape.Paths.Count);
            var outline = shape.Paths[0];
            Assert.Equal((1, 0, 1), (outline.FillLeft, outline.FillRight, outline.Line));
            Assert.Equal(new SwfPoint(0, 0), outline.Start);
            Assert.Equal([new SwfPoint(400, 0), new SwfPoint(400, 200), new SwfPoint(0, 200), new SwfPoint(0, 0)], outline.Edges.Select(e => e.End));
            Assert.All(outline.Edges, edge => Assert.Null(edge.Control));
            var inner = shape.Paths[1];
            Assert.Equal((0, 1, 0), (inner.FillLeft, inner.FillRight, inner.Line));
            Assert.Equal(new SwfPoint(100, 50), inner.Start);
            Assert.Equal(new SwfPoint(120, 50), Assert.Single(inner.Edges).End);
        }

        [Fact]
        public void Read_ShapeOutline_StylesStayWhileOnlyThePenMoves()
        {
            var shape = ReadPathShape(null, edges => edges
                .Style(0, 0, fillRight: 1).Line(100, 0)
                .Style(500, 500).Line(0, 100));

            Assert.All(shape.Paths, path => Assert.Equal((0, 1, 0), (path.FillLeft, path.FillRight, path.Line)));
            Assert.Equal(new SwfPoint(500, 500), shape.Paths[1].Start);
        }

        [Fact]
        public void Read_ShapeOutline_CurveKeepsItsControlPoint()
        {
            var shape = ReadPathShape(null, edges => edges.Style(10, 20, fillLeft: 1).Curve(30, -10, 30, 10));

            var curve = Assert.Single(Assert.Single(shape.Paths).Edges);
            Assert.Equal(new SwfPoint(40, 10), curve.Control);
            Assert.Equal(new SwfPoint(70, 20), curve.End);
        }

        [Fact]
        public void Read_ShapeOutline_NewStyleArraysAreNumberedAfterTheEarlierOnes()
        {
            var shape = ReadPathShape(null, edges => edges
                .Style(0, 0, fillLeft: 1).Line(10, 0)
                .Style(0, 50, fillLeft: 2, newFills: [new SwfColor(1, 2, 3, 4), new SwfColor(5, 6, 7, 8)]).Line(10, 0));

            Assert.Equal(3, shape.Fills.Count);
            Assert.Equal(1, shape.Paths[0].FillLeft);
            Assert.Equal(3, shape.Paths[1].FillLeft);
            Assert.Equal(new SwfColor(5, 6, 7, 8), shape.Fills[2].Color);
        }

        [Fact]
        public void Read_Sprite_GivesItsOwnTimeline()
        {
            var movie = Read(swf => swf.Sprite(4, t => t.Label("in").Place(new SwfPlacement(1) { Character = 9 }).ShowFrame().Remove(1).ShowFrame(), declaredFrameCount: 2));

            var sprite = Assert.IsType<SwfSprite>(movie.Characters[4]);
            Assert.Equal(2, sprite.Timeline.DeclaredFrameCount);
            Assert.Equal(2, sprite.Timeline.Frames.Count);
            Assert.Equal(["in"], sprite.Timeline.Frames[0].Labels);
            Assert.IsType<SwfPlaceObject>(Assert.Single(sprite.Timeline.Frames[0].Actions));
            Assert.Equal(new SwfRemoveObject(1), Assert.Single(sprite.Timeline.Frames[1].Actions));
        }

        [Fact]
        public void Read_EditText_GivesItsFontColourAndText()
        {
            var movie = Read(swf => swf.EditText(6, new TwipRect(0, 0, 2000, 400), fontClass: "$Main", fontHeight: 320, text: "<p>hi</p>", html: true,
                color: new SwfColor(1, 2, 3, 4), variableName: "v"));

            var text = Assert.IsType<SwfEditText>(movie.Characters[6]);
            Assert.Equal(new TwipRect(0, 0, 2000, 400), text.Bounds);
            Assert.Equal("$Main", text.FontClass);
            Assert.Equal(320, text.FontHeight);
            Assert.Equal(new SwfColor(1, 2, 3, 4), text.TextColor);
            Assert.Equal("<p>hi</p>", text.InitialText);
            Assert.True(text.Flags.Html);
            Assert.Equal("v", text.VariableName);
        }

        [Fact]
        public void Read_FontWithoutGlyphs_GivesItsNameAndStyle()
        {
            var movie = Read(swf => swf.Font(8, "Some Family", bold: true));

            var font = Assert.IsType<SwfFont>(movie.Characters[8]);
            Assert.Equal("Some Family", font.Name);
            Assert.True(font.Bold);
            Assert.False(font.Italic);
            Assert.Empty(font.Codes);
        }

        [Fact]
        public void Read_Linkage_GivesSymbolClassesExportsImportsAndScalingGrids()
        {
            var movie = Read(swf => swf
                .SymbolClass((1, "A.B"), (0, "Main"))
                .Export((2, "Exported"))
                .Import("fonts_en.swf", (3, "$Main"))
                .ScalingGrid(2, new TwipRect(20, 20, 200, 100)));

            Assert.Equal([(1, "A.B"), (0, "Main")], movie.SymbolClasses);
            Assert.Equal([(2, "Exported")], movie.Exports);
            Assert.Equal(new SwfImport("fonts_en.swf", "$Main"), movie.Imports[3]);
            Assert.Equal(new TwipRect(20, 20, 200, 100), movie.ScalingGrids[2]);
        }

        [Fact]
        public void Read_ActionScriptBlocks_AreKeptByNameInOrder()
        {
            var movie = Read(swf => swf.Abc("first", [1, 2]).Abc("second", [3]));

            Assert.Equal(["first", "second"], movie.AbcBlocks.Select(b => b.Name));
            Assert.Equal([3], movie.AbcBlocks[1].Data);
        }

        [Fact]
        public void Read_PlaceObject4_Throws()
        {
            Assert.Throws<SwfFormatException>(() => Read(swf => swf.Root.Tag(SwfTagCodes.PlaceObject4, [0, 0, 1, 0])));
        }

        [Fact]
        public void Read_PlaceObject1_IsAPlacementWithACharacterAndMatrix()
        {
            byte[] body = [.. new SwfBitWriter().U16(7).U16(2).Matrix(SwfPlacement.Translate(40, 20)).ToArray()];

            var place = Assert.IsType<SwfPlaceObject>(Assert.Single(Read(swf => swf.Root.Tag(SwfTagCodes.PlaceObject, body)).Root.Frames[0].Actions));

            Assert.Equal(new SwfPlaceObject(2, Move: false, CharacterId: 7, Matrix: SwfPlacement.Translate(40, 20)), place);
        }

        [Fact]
        public void Read_UnknownBlendMode_Throws()
        {
            Assert.Throws<SwfFormatException>(() => Read(swf => swf.Root.Place(new SwfPlacement(1) { Character = 1, BlendMode = 15 })));
        }

        private static SwfPlaceObject OnlyPlacement(SwfMovie movie) => Assert.IsType<SwfPlaceObject>(Assert.Single(movie.Root.Frames[0].Actions));

        [Fact]
        public void Read_BevelFilter_StatesTheHighlightBeforeTheShadow()
        {
            byte[] filters = [.. new SwfBitWriter().U8(1).U8(3).Rgba(255, 255, 255, 255).Rgba(0, 0, 0, 255)
                .Fixed(2).Fixed(2).Fixed(0.5).Fixed(4).U16(256).Flag(false).Flag(false).Flag(true).Flag(false).UB(1, 4).ToArray()];

            var place = OnlyPlacement(Read(swf => swf.Root.Place(new SwfPlacement(1) { Character = 1, Filters = filters })));

            var bevel = Assert.IsType<BevelFilter>(Assert.Single(place.Filters!));
            Assert.Equal(new SwfColor(255, 255, 255, 255), bevel.HighlightColor);
            Assert.Equal(new SwfColor(0, 0, 0, 255), bevel.ShadowColor);
        }

        [Fact]
        public void Read_PlaceObject3ImageWithoutACharacter_NamesTheBitmapClass()
        {
            byte[] body = [.. new SwfBitWriter().U8(0x00).U8(0x10).U16(3).String("Picture").ToArray()];

            var place = OnlyPlacement(Read(swf => swf.Root.Tag(SwfTagCodes.PlaceObject3, body)));

            Assert.Equal("Picture", place.ClassName);
            Assert.Null(place.CharacterId);
        }

        [Fact]
        public void Read_PlaceObject3ImageWithACharacter_HasNoClassNameField()
        {
            byte[] body = [.. new SwfBitWriter().U8(0x02).U8(0x10).U16(3).U16(5).ToArray()];

            var place = OnlyPlacement(Read(swf => swf.Root.Tag(SwfTagCodes.PlaceObject3, body)));

            Assert.Null(place.ClassName);
            Assert.Equal(5, place.CharacterId);
        }

        [Fact]
        public void Read_PlaceObject3OpaqueBackground_IsKept()
        {
            byte[] body = [.. new SwfBitWriter().U8(0x02).U8(0x40).U16(1).U16(7).Rgba(10, 20, 30, 255).ToArray()];

            Assert.Equal(new SwfColor(10, 20, 30, 255), OnlyPlacement(Read(swf => swf.Root.Tag(SwfTagCodes.PlaceObject3, body))).Background);
        }

        [Fact]
        public void Read_CsmTextSettings_AreKeptByText()
        {
            byte[] body = [.. new SwfBitWriter().U16(6).UB(1, 2).UB(2, 3).UB(0, 3).Bytes(BitConverter.GetBytes(-50f)).Bytes(BitConverter.GetBytes(200f)).U8(0).ToArray()];

            var movie = Read(swf => swf.Tag(SwfTagCodes.CsmTextSettings, body));

            Assert.Equal(new SwfTextSettings(1, 2, -50f, 200f), movie.TextSettings[6]);
        }

        [Fact]
        public void Filters_WithEqualContents_AreEqual()
        {
            Assert.Equal(new ColorMatrixFilter(new EquatableList<float>([1, 0, 0])), new ColorMatrixFilter(new EquatableList<float>([1, 0, 0])));
            Assert.NotEqual(new ColorMatrixFilter(new EquatableList<float>([1, 0, 0])), new ColorMatrixFilter(new EquatableList<float>([0, 1, 0])));
        }

        [Fact]
        public void Read_UnneededTags_AreSkipped()
        {
            var movie = Read(swf => swf.Tag(777, [1, 2, 3, 4, 5]).Shape(1, new TwipRect(0, 0, 1, 1), new SwfColor(0, 0, 0, 255)));

            Assert.IsType<SwfShape>(Assert.Single(movie.Characters).Value);
        }
    }
}
