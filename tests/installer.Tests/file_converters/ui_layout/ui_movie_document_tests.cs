using System.Text.Json;
using AssetConverter;
using AssetConverter.Abc;
using AssetConverter.Swf;
using AssetConverter.UiLayout;
using Xunit;

namespace InstallerTests.UiLayout
{
    public sealed class UiMovieDocumentTests
    {
        private const int Square = 1;
        private const int Label = 2;
        private const int Panel = 3;
        private const int WindingShape = 9;

        private static readonly ConversionStamp Stamp = UiDocumentHeader.Stamp(null, SourceFingerprint.Of(@"Interface\Synth.swf", [9, 9]));

        private static SwfPlacement At(int depth, int character, string? name = null, double x = 0, double y = 0) =>
            new(depth) { Character = character, Name = name, Matrix = SwfPlacement.Translate(x, y) };

        /// <summary>
        /// A movie whose main timeline places the panel sprite as <c>menu</c>. The panel shows <c>bg</c>, <c>label</c> and two objects
        /// named <c>twin</c> at frame 1, fades <c>bg</c> at its labelled frame 2 and adds <c>extra</c> at frame 3. Its class has a
        /// constant and a frame script.
        /// </summary>
        private static byte[] SynthMovie()
        {
            var swf = new SwfMovieBuilder { Stage = new TwipRect(0, 0, 19200, 10800), FrameRate = 30 };
            swf.Shape(Square, new TwipRect(0, 0, 200, 200), new SwfColor(255, 0, 0, 255));
            swf.Shape(WindingShape, new TwipRect(0, 0, 100, 100), new SwfColor(0, 255, 0, 255), version: 4, shapeFlags: 0x06);
            swf.EditText(Label, new TwipRect(0, 0, 2000, 400), fontClass: "$Main", fontHeight: 400, html: true,
                text: "<p align=\"left\"><font face=\"$Main\" size=\"20\" letterSpacing=\"1.5\">Hi</font></p>");
            swf.Sprite(Panel, t => t
                .Place(At(1, Square, "bg"))
                .Place(At(2, Label, "label", 100, 0))
                .Place(At(4, Square, "twin"))
                .Place(At(5, Square, "twin"))
                .ShowFrame()
                .Label("open")
                .Place(new SwfPlacement(1) { Move = true, Color = SwfPlacement.Alpha(0.5) })
                .ShowFrame()
                .Place(At(3, Square, "extra"))
                .ShowFrame()
                .ShowFrame());
            swf.SymbolClass((Panel, "Lib.Panel"), (0, "MainDoc"));
            swf.Abc("synth", PanelScripts());
            swf.Root.Label("intro").Place(At(1, Panel, "menu", 2000, 1000)).ShowFrame();
            return swf.Build();
        }

        private static byte[] PanelScripts()
        {
            var abc = new AbcBuilder();
            int frame3 = abc.Function(new AbcCode().GetLocal0().PushScope()
                .FindPropStrict(abc.Name("gotoAndPlay")).PushByte(2).CallPropVoid(abc.Name("gotoAndPlay"), 1).ReturnVoid().ToArray());
            abc.Class("Lib", "Panel", abc.Name("Base", "Lib"), abc.Method(), abc.Method(),
                [new AbcMethodTraitSpec(abc.Name("frame3"), frame3)],
                [new AbcSlotSpec(abc.Name("SPEED"), true, abc.Int(4), AbcBuilder.ConstantInt)]);
            return abc.Build();
        }

        private static byte[] Convert(byte[] swf)
        {
            SwfMovie movie = SwfMovieReader.Read(swf);
            var scripts = AbcConstantExtractor.Extract(movie.AbcBlocks.Select(b => (b.Name, AbcReader.Read(b.Data))).ToList(), "Synth");
            return UiMovieDocument.Write("Synth", Stamp, movie, scripts);
        }

        private static JsonDocument Write(byte[] swf) => JsonDocument.Parse(Convert(swf));

        /// <summary>The extractor version and the SHA-256 of the synthetic movie's document it writes.</summary>
        private static readonly (int ExtractorVersion, string Sha256) Golden = (5, "a7a2f8c32a030ecebb42ca0dc3cdd950bb831fa96f78fef9377d3e8da789aade");

        [Fact]
        public void Write_ChangedOutput_RaisesTheExtractorVersion()
        {
            string written = System.Convert.ToHexString(System.Security.Cryptography.SHA256.HashData(Convert(SynthMovie()))).ToLowerInvariant();

            Assert.True(Golden == (UiDocumentHeader.ExtractorVersion, written),
                $"The movie document changed (SHA-256 {written}). Installed documents are reused unless UiDocumentHeader.ExtractorVersion is raised: "
                + "raise it and record the new version and hash in Golden.");
        }

        [Fact]
        public void Write_ShapeVersion4_GivesItsFlagsBesideThePaths()
        {
            var shape = Write(SynthMovie()).RootElement.GetProperty("characters").GetProperty(WindingShape.ToString(System.Globalization.CultureInfo.InvariantCulture));

            Assert.True(shape.GetProperty("fillWindingRule").GetBoolean());
            Assert.True(shape.GetProperty("nonScalingStrokes").GetBoolean());
            Assert.False(shape.GetProperty("scalingStrokes").GetBoolean());
        }

        [Fact]
        public void Write_ShapeOutline_GivesPixelPathsWithTheirStyleNumbers()
        {
            var swf = new SwfMovieBuilder();
            swf.PathShape(1, new TwipRect(0, 0, 400, 200), new SwfColor(255, 255, 255, 255), null, 0, edges => edges
                .Style(10, 20, fillLeft: 1).Line(40, 0).Curve(20, 0, 0, 20));
            swf.Root.ShowFrame();

            var path = Write(swf.Build()).RootElement.GetProperty("characters").GetProperty("1").GetProperty("paths").EnumerateArray().Single();

            Assert.Equal(1, path.GetProperty("fillLeft").GetInt32());
            Assert.Equal(0, path.GetProperty("fillRight").GetInt32());
            Assert.Equal(0, path.GetProperty("line").GetInt32());
            Assert.Equal([0.5, 1], Numbers(path.GetProperty("start")));
            var edges = path.GetProperty("edges").EnumerateArray().Select(Numbers).ToArray();
            Assert.Equal([2.5, 1], edges[0]);
            Assert.Equal([3.5, 1, 3.5, 2], edges[1]);
        }

        private static readonly Lazy<JsonDocument> Document = new(() => Write(SynthMovie()));

        private static JsonElement Root => Document.Value.RootElement;

        private static JsonElement Instance(string path) => Root.GetProperty("instances").GetProperty(path);

        private static double[] Numbers(JsonElement array) => array.EnumerateArray().Select(e => e.GetDouble()).ToArray();

        [Fact]
        public void Write_Header_NamesTheKindAndConversion()
        {
            Assert.Equal(UiDocumentHeader.Format, Root.GetProperty("format").GetInt32());
            Assert.Equal("movie", Root.GetProperty("kind").GetString());
            Assert.Equal(Stamp.Sources[0].Sha256, Root.GetProperty("conversion").GetProperty("sources")[0].GetProperty("sha256").GetString());
            Assert.Equal("Synth", Root.GetProperty("movie").GetString());
        }

        [Fact]
        public void Write_Movie_GivesStageInPixelsFrameRateDocumentClassAndLabels()
        {
            Assert.Equal([0, 0, 960, 540], Numbers(Root.GetProperty("stage")));
            Assert.Equal(30, Root.GetProperty("fps").GetDouble());
            Assert.Equal("MainDoc", Root.GetProperty("documentClass").GetString());
            Assert.Equal(1, Root.GetProperty("frameCount").GetInt32());
            Assert.Equal(1, Root.GetProperty("labels").GetProperty("intro").GetInt32());
        }

        [Fact]
        public void Write_Symbols_MapEachClassToItsCharacter()
        {
            Assert.Equal(Panel, Root.GetProperty("symbols").GetProperty("Lib.Panel").GetInt32());
            Assert.Equal("Lib.Panel", Root.GetProperty("characters").GetProperty("3").GetProperty("class").GetString());
        }

        [Fact]
        public void Write_Characters_DescribeShapesSpritesAndTextFields()
        {
            JsonElement characters = Root.GetProperty("characters");
            Assert.Equal("shape", characters.GetProperty("1").GetProperty("kind").GetString());
            Assert.Equal([0, 0, 10, 10], Numbers(characters.GetProperty("1").GetProperty("bounds")));
            Assert.Equal(4, characters.GetProperty("3").GetProperty("frameCount").GetInt32());
            Assert.Equal(2, characters.GetProperty("3").GetProperty("labels").GetProperty("open").GetInt32());

            JsonElement text = characters.GetProperty("2");
            Assert.Equal("$Main", text.GetProperty("font").GetString());
            Assert.Equal(20, text.GetProperty("size").GetDouble());
            Assert.True(text.GetProperty("flags").GetProperty("html").GetBoolean());
            Assert.Equal(1.5, text.GetProperty("htmlFormat").GetProperty("FONT.LETTERSPACING").GetDouble());
            Assert.Equal("left", text.GetProperty("htmlFormat").GetProperty("P.ALIGN").GetString());
        }

        [Fact]
        public void Write_Instances_ArePathsOfNamesWithTheirLocalAndComposedPlacement()
        {
            JsonElement label = Instance("menu/label");

            Assert.Equal(Label, label.GetProperty("char").GetInt32());
            Assert.Equal("text", label.GetProperty("kind").GetString());
            Assert.Equal([1, 0, 0, 1, 5, 0], Numbers(label.GetProperty("local").GetProperty("matrix")));
            Assert.Equal([1, 0, 0, 1, 105, 50], Numbers(label.GetProperty("world").GetProperty("matrix")));
            Assert.Equal([1, 4], Numbers(label.GetProperty("frames")));
        }

        [Fact]
        public void Write_LabelledFrame_IsAViewOfEveryObjectThere()
        {
            Assert.Equal(0.5, Numbers(Instance("menu:open/bg").GetProperty("local").GetProperty("color").GetProperty("mult"))[3]);
            Assert.True(Root.GetProperty("instances").TryGetProperty("menu:open/label", out _));
        }

        [Fact]
        public void Write_UnlabelledKeyframe_IsAViewOfTheObjectsAppearingThere()
        {
            JsonElement instances = Root.GetProperty("instances");

            Assert.Equal([3, 4], Numbers(Instance("menu:3/extra").GetProperty("frames")));
            Assert.False(instances.TryGetProperty("menu:3/bg", out _));
            Assert.False(instances.TryGetProperty("menu:4/extra", out _));
        }

        [Fact]
        public void Write_PlacementChangingDuringItsLife_HasATrack()
        {
            var track = Instance("menu/bg").GetProperty("track").EnumerateArray().ToList();

            Assert.Equal([1, 2], track.Select(k => k.GetProperty("frame").GetInt32()));
            Assert.Equal(0.5, Numbers(track[1].GetProperty("color").GetProperty("mult"))[3]);
            Assert.False(Instance("menu/label").TryGetProperty("track", out _));
        }

        [Fact]
        public void Write_TwoObjectsWithOneName_AreUnresolvedByNameAndReachableByDepth()
        {
            Assert.Equal("2 instances are named 'twin' at frame 1", Root.GetProperty("unresolved").GetProperty("menu/twin").GetString());
            Assert.Equal("twin", Instance("menu/@4").GetProperty("name").GetString());
            Assert.Equal("twin", Instance("menu/@5").GetProperty("name").GetString());
        }

        [Fact]
        public void Write_ExportedSprite_IsARootOfItsOwn()
        {
            Assert.Equal([1, 0, 0, 1, 0, 0], Numbers(Instance("[Lib.Panel]/bg").GetProperty("world").GetProperty("matrix")));
        }

        [Fact]
        public void Write_ArrayOfLookedUpClasses_IsAReferenceListNotAConstant()
        {
            JsonElement document = Write(SyntheticArtArchive.WithMarkerClasses("First", "Second")).RootElement;

            Assert.Equal(["First", "Second"],
                document.GetProperty("referenceLists").GetProperty("MapHolder.MarkerTypesA").EnumerateArray().Select(e => e.GetString()));
            Assert.False(document.GetProperty("constants").TryGetProperty("MapHolder.MarkerTypesA", out _));
        }

        [Fact]
        public void Write_ImportedCharacter_HasTheImportKind()
        {
            var swf = new SwfMovieBuilder();
            swf.Import("shared.swf", (5, "Shared"));
            swf.Root.Place(At(1, 5, "shared")).ShowFrame();

            JsonElement document = Write(swf.Build()).RootElement;

            Assert.Equal("import", document.GetProperty("instances").GetProperty("shared").GetProperty("kind").GetString());
        }

        [Fact]
        public void Write_PlacedCharacterTheMovieNeitherDefinesNorImports_Throws()
        {
            var swf = new SwfMovieBuilder();
            swf.Root.Place(At(1, 77, "ghost")).ShowFrame();

            var failure = Assert.Throws<SwfFormatException>(() => Convert(swf.Build()));

            Assert.Contains("77", failure.Message);
        }

        [Fact]
        public void Write_SpriteThatContainsItself_Throws()
        {
            var swf = new SwfMovieBuilder();
            swf.Sprite(3, t => t.Place(At(1, 3, "again")).ShowFrame());
            swf.Root.Place(At(1, 3, "start")).ShowFrame();

            Assert.Throws<SwfFormatException>(() => Convert(swf.Build()));
        }

        [Fact]
        public void Write_ClassExportedForTwoCharacters_Throws()
        {
            var swf = new SwfMovieBuilder();
            swf.Shape(1, new TwipRect(0, 0, 20, 20), new SwfColor(0, 0, 0, 255));
            swf.Shape(2, new TwipRect(0, 0, 20, 20), new SwfColor(0, 0, 0, 255));
            swf.SymbolClass((1, "Twin"), (2, "Twin"));
            swf.Root.ShowFrame();

            var failure = Assert.Throws<SwfFormatException>(() => Convert(swf.Build()));

            Assert.Contains("Twin", failure.Message);
        }

        [Fact]
        public void Write_Scripts_GiveConstantsAndLiterals()
        {
            Assert.Equal(4, Root.GetProperty("constants").GetProperty("Lib.Panel.SPEED").GetInt32());
            Assert.Equal([2], Numbers(Root.GetProperty("literals").GetProperty("Lib.Panel.frame3#gotoAndPlay(0)").GetProperty("values")));
        }

        [Fact]
        public void Write_SuperClasses_NameTheClassEachClassExtends()
        {
            Assert.Equal("Lib.Base", Root.GetProperty("superClasses").GetProperty("Lib.Panel").GetString());
        }

        [Fact]
        public void Write_IsDeterministic()
        {
            SwfMovie movie = SwfMovieReader.Read(SynthMovie());
            var scripts = AbcConstantExtractor.Extract(movie.AbcBlocks.Select(b => (b.Name, AbcReader.Read(b.Data))).ToList(), "Synth");

            Assert.Equal(UiMovieDocument.Write("Synth", Stamp, movie, scripts), UiMovieDocument.Write("Synth", Stamp, movie, scripts));
        }
    }
}
