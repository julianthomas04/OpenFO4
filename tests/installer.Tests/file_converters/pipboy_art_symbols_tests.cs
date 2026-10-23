using AssetConverter;
using AssetConverter.Swf;
using Xunit;

namespace InstallerTests
{
    // Symbols are found by class, class prefix, instance name or frame label, and the
    // doll pieces and condition movies keep the placements and bounds the movie gives them.
    public sealed class PipboyArtSymbolsTests
    {
        static readonly SwfColor Black = new(0, 0, 0, 255);

        static PipboySwfTree Tree(Action<SwfMovieBuilder> build)
        {
            var swf = new SwfMovieBuilder();
            build(swf);
            return new PipboySwfTree(swf.Read());
        }

        static SwfPlacement At(int character, int depth, double xTwips = 0, double yTwips = 0, string? name = null) =>
            new(depth) { Character = character, Matrix = SwfPlacement.Translate(xTwips, yTwips), Name = name };

        static SwfMovieBuilder Box(SwfMovieBuilder swf, int id, double xMin = 0, double yMin = 0, double xMax = 20, double yMax = 20) =>
            swf.Shape(id, new TwipRect(xMin, yMin, xMax, yMax), Black);

        static ColorTransform GreyMultiplier(int term) => ColorTransform.FromTerms([term, term, term, 256], [0, 0, 0, 0]);

        [Fact]
        public void CharacterOfClassPrefix_FindsTheOnlyAutoNumberedClass()
        {
            var tree = Tree(swf => swf.SymbolClass((48, "Page_fla.DamageIcons_24"), (49, "Other")));

            Assert.Equal(48, tree.CharacterOfClassPrefix("Page_fla.DamageIcons_"));
        }

        [Fact]
        public void CharacterOfClassPrefix_ZeroOrSeveralMatches_Throw()
        {
            var tree = Tree(swf => swf.SymbolClass((1, "A_1"), (2, "A_2")));

            Assert.Throws<SwfFormatException>(() => tree.CharacterOfClassPrefix("B_"));
            Assert.Throws<SwfFormatException>(() => tree.CharacterOfClassPrefix("A_"));
            Assert.Throws<SwfFormatException>(() => tree.CharacterOfClass("Missing"));
        }

        [Fact]
        public void Sequence_EveryFrame_RunsFromTheFirstToTheLastFrameOfTheSymbol()
        {
            var tree = Tree(swf => Box(swf, 1)
                .Sprite(2, t => t.Place(At(1, 1)).ShowFrame().ShowFrame().ShowFrame())
                .SymbolClass((2, "Page_fla.Strip_56")));

            var resolved = PipboyArtSymbols.Sequence(
                new NamedSymbolClipSource("icon/strip", "Page.swf", new SymbolRef.ByClassPrefix("Page_fla.Strip_"), FrameSelection.EveryFrame), tree);

            Assert.Equal(2, resolved.CharacterId);
            Assert.Equal([1, 2, 3], resolved.Frames);
        }

        [Fact]
        public void Sequence_LabelledFrame_IsTheFrameTheLabelNames()
        {
            var tree = Tree(swf => swf.Sprite(2, t => t.ShowFrame().Label("Second").ShowFrame().Label("Third").ShowFrame()).SymbolClass((2, "Icons")));

            var resolved = PipboyArtSymbols.Sequence(
                new NamedSymbolClipSource("icon/x", "Page.swf", new SymbolRef.ByClass("Icons"), new FrameSelection.Labelled("Third")), tree);

            Assert.Equal([3], resolved.Frames);
            Assert.Throws<SwfFormatException>(() => PipboyArtSymbols.Sequence(
                new NamedSymbolClipSource("icon/x", "Page.swf", new SymbolRef.ByClass("Icons"), new FrameSelection.Labelled("Missing")), tree));
        }

        static PipboySwfTree IconsWithTextFrames(params bool[] frameShowsText) =>
            Tree(swf => Box(swf, 1)
                .EditText(2, new TwipRect(0, 0, 10, 10))
                .Sprite(3, t =>
                {
                    foreach (bool text in frameShowsText)
                        t.Remove(1).Place(At(text ? 2 : 1, 1)).ShowFrame();
                })
                .SymbolClass((3, "Icons")));

        static NamedSymbolClipSource DroppingTextFrames() =>
            new("icon/strip", "Page.swf", new SymbolRef.ByClass("Icons"), new FrameSelection.UntilTrailingText());

        [Fact]
        public void Sequence_TrailingTextFrames_AreLeftOut()
        {
            var resolved = PipboyArtSymbols.Sequence(DroppingTextFrames(), IconsWithTextFrames(false, false, false, true));

            Assert.Equal([1, 2, 3], resolved.Frames);
        }

        [Fact]
        public void Sequence_WithoutTextFrames_KeepsEveryFrame()
        {
            Assert.Equal([1, 2], PipboyArtSymbols.Sequence(DroppingTextFrames(), IconsWithTextFrames(false, false)).Frames);
        }

        [Fact]
        public void Sequence_TextFrameBeforeArt_Throws()
        {
            Assert.Throws<SwfFormatException>(() => PipboyArtSymbols.Sequence(DroppingTextFrames(), IconsWithTextFrames(false, true, false)));
        }

        [Fact]
        public void Sequence_FrameRangePastTheEndOfTheSymbol_Throws()
        {
            var tree = Tree(swf => swf.Sprite(2, t => t.ShowFrame()).SymbolClass((2, "Icons")));

            Assert.Throws<SwfFormatException>(() => PipboyArtSymbols.Sequence(
                new NamedSymbolClipSource("icon/x", "Page.swf", new SymbolRef.ByClass("Icons"), new FrameSelection.Only(7)), tree));
        }

        static PipboySwfTree HolderTree() =>
            Tree(swf => Box(swf, 62)
                .Sprite(63, t => t.Place(At(62, 1)).ShowFrame())
                .Sprite(32, t => t.ShowFrame())
                .Sprite(60, t => t.ShowFrame())
                .Sprite(64, t => t.Place2(At(32, 1, 0, 0, "Backdrop_mc")).Place2(At(60, 3, 400, 0, name: "Label_tf")).Place(At(63, 5, 300, 0)).ShowFrame())
                .SymbolClass((64, "Holder")));

        [Fact]
        public void OnlyUnnamedChild_IsTheGlyphNextToNamedParts_WhateverItsDepth()
        {
            var tree = HolderTree();

            Assert.Equal(63, PipboyArtSymbols.Character(new SymbolRef.OnlyUnnamedChild(new SymbolRef.ByClass("Holder")), tree));
        }

        [Fact]
        public void OnlyUnnamedChild_NoneOrSeveral_Throws()
        {
            var none = Tree(swf => swf.Sprite(32, t => t.ShowFrame()).Sprite(64, t => t.Place2(At(32, 1, name: "A_mc")).ShowFrame()));
            var several = Tree(swf => swf.Sprite(32, t => t.ShowFrame()).Sprite(64, t => t.Place(At(32, 1)).Place(At(32, 2, 10)).ShowFrame()));

            Assert.Throws<SwfFormatException>(() => none.OnlyUnnamedChild(64));
            Assert.Throws<SwfFormatException>(() => several.OnlyUnnamedChild(64));
        }

        static PipboySwfTree BarTree(Action<SwfMovieBuilder> shapes, Func<SwfTimelineWriter, SwfTimelineWriter> bar) =>
            Tree(swf =>
            {
                shapes(swf);
                swf.Sprite(32, t => t.ShowFrame()).Sprite(64, t => bar(t).ShowFrame()).SymbolClass((64, "Bar"));
            });

        [Fact]
        public void UnnamedBeside_IsTheNearestGlyphOfTheLabel_AndIgnoresOtherGlyphs()
        {
            var tree = BarTree(swf => { Box(swf, 1); Box(swf, 2); Box(swf, 3); }, t => t
                .Place(At(1, 1, 0, 0)).Place(At(2, 2, 400, 0)).Place(At(3, 3, 5000, 0)).Place2(At(32, 4, 500, 0, name: "Weight_tf")));

            Assert.Equal(2, PipboyArtSymbols.Character(new SymbolRef.UnnamedBeside(new SymbolRef.ByClass("Bar"), "Weight_tf"), tree));
        }

        [Fact]
        public void UnnamedBeside_TiedCandidates_Throw()
        {
            var tree = BarTree(swf => { Box(swf, 1); Box(swf, 2); }, t => t
                .Place(At(1, 1, 0, 0)).Place(At(2, 2, 200, 0)).Place2(At(32, 3, 100, 0, name: "Weight_tf")));

            Assert.Throws<SwfFormatException>(() => tree.UnnamedBeside(64, "Weight_tf"));
        }

        [Fact]
        public void UnnamedBeside_CharacterPlacedTwice_IsATileNotAGlyph()
        {
            var tree = BarTree(swf => { Box(swf, 1); Box(swf, 2); }, t => t
                .Place(At(1, 1, 90, 0)).Place(At(1, 2, 110, 0)).Place(At(2, 3, 900, 0)).Place2(At(32, 4, 100, 0, name: "Weight_tf")));

            Assert.Equal(2, tree.UnnamedBeside(64, "Weight_tf").CharacterId);
        }

        [Fact]
        public void UnnamedBeside_NoCandidateOrNoSibling_Throws()
        {
            var tree = BarTree(_ => { }, t => t.Place2(At(32, 1, name: "Weight_tf")));

            Assert.Throws<SwfFormatException>(() => tree.UnnamedBeside(64, "Weight_tf"));
            Assert.Throws<SwfFormatException>(() => tree.UnnamedBeside(64, "Missing_tf"));
        }

        [Fact]
        public void Character_PathFollowsInstanceNames()
        {
            var tree = Tree(swf => Box(swf, 1)
                .Sprite(17, t => t.Place(At(1, 1)).ShowFrame())
                .Sprite(28, t => t.Place(At(17, 1)).Place(At(1, 3, name: "Card_mc")).ShowFrame())
                .Sprite(44, t => t.Place(At(28, 5, name: "Holder_mc")).ShowFrame())
                .SymbolClass((44, "Grid")));
            var path = new SymbolRef.OnlyUnnamedChild(new SymbolRef.NamedChild(new SymbolRef.ByClass("Grid"), "Holder_mc"));

            Assert.Equal(17, PipboyArtSymbols.Character(path, tree));
            Assert.Throws<SwfFormatException>(() => PipboyArtSymbols.Character(new SymbolRef.NamedChild(new SymbolRef.ByClass("Grid"), "Missing_mc"), tree));
        }

        [Fact]
        public void AnimatedContent_DescendsThroughSingleFrameWrappers_AndStopsAtTheAnimation()
        {
            var tree = Tree(swf => swf
                .Sprite(31, t => t.ShowFrame().ShowFrame().ShowFrame())
                .Sprite(32, t => t.Place(At(31, 1)).ShowFrame())
                .SymbolClass((32, "Quest_DefaultBoy")));

            Assert.Equal(31, tree.AnimatedContent(tree.CharacterOfClass("Quest_DefaultBoy")));
            Assert.Equal(31, tree.AnimatedContent(31));
        }

        static PipboySwfTree DollTree(SwfMatrix? torsoMatrix = null) =>
            Tree(swf => Box(swf, 10, xMax: 10, yMax: 10)
                .Shape(12, new TwipRect(0, 0, 10, 10), Black)
                .Sprite(11, t => t.ShowFrame())
                .Sprite(13, t => t
                    .Label("LabelA").Place2(At(10, 1, 25, 3) with { Color = GreyMultiplier(200) })
                    .Place2(At(11, 3, name: "Part_mc") with { Matrix = torsoMatrix ?? SwfPlacement.Translate(60, 40) }).ShowFrame()
                    .Remove(1).Label("LabelB").Place2(At(12, 1)).ShowFrame())
                .SymbolClass((13, "Host")));

        [Fact]
        public void DollPiece_ByInstanceName_CarriesThePlacementOffsetInPixels()
        {
            var piece = new DollPieceSource("part/a", "Page.swf", "Host", "Part_mc", null);

            var resolved = PipboyArtSymbols.DollPiece(piece, DollTree());

            Assert.Equal(11, resolved.CharacterId);
            Assert.Equal([3, 2], resolved.Offset);
            Assert.Null(resolved.Tint);
        }

        [Fact]
        public void DollPiece_ByFrameLabel_CarriesOffsetAndColourMultiplier()
        {
            var piece = new DollPieceSource("part/b", "Page.swf", "Host", null, "LabelA");

            var resolved = PipboyArtSymbols.DollPiece(piece, DollTree());

            Assert.Equal(10, resolved.CharacterId);
            Assert.Equal([1.25, 0.15], resolved.Offset);
            Assert.Equal([200 / 256.0, 200 / 256.0, 200 / 256.0, 1.0], resolved.Tint);
        }

        [Fact]
        public void DollPiece_ModifiedLaterInItsFrame_CarriesThePlayedState()
        {
            var tree = Tree(swf => swf
                .Sprite(11, t => t.ShowFrame())
                .Sprite(13, t => t
                    .Place2(At(11, 3, 60, 40, name: "Part_mc"))
                    .Place2(new SwfPlacement(3) { Move = true, Color = GreyMultiplier(128) }).ShowFrame())
                .SymbolClass((13, "Host")));

            var resolved = PipboyArtSymbols.DollPiece(new DollPieceSource("part/a", "Page.swf", "Host", "Part_mc", null), tree);

            Assert.Equal([3, 2], resolved.Offset);
            Assert.Equal([0.5, 0.5, 0.5, 1.0], resolved.Tint);
        }

        [Fact]
        public void DollPiece_ColourTransformThatAddsToAChannel_Throws()
        {
            var tree = Tree(swf => swf
                .Sprite(11, t => t.ShowFrame())
                .Sprite(13, t => t
                    .Place2(At(11, 3, name: "Part_mc") with { Color = ColorTransform.FromTerms([256, 256, 256, 256], [10, 0, 0, 0]) }).ShowFrame())
                .SymbolClass((13, "Host")));

            Assert.Throws<SwfFormatException>(() =>
                PipboyArtSymbols.DollPiece(new DollPieceSource("part/a", "Page.swf", "Host", "Part_mc", null), tree));
        }

        [Fact]
        public void DollPiece_PlacedWithAScale_Throws()
        {
            var tree = DollTree(SwfMatrix.Identity with { A = 2, D = 2, Tx = 60, Ty = 40 });
            var piece = new DollPieceSource("part/a", "Page.swf", "Host", "Part_mc", null);

            Assert.Throws<SwfFormatException>(() => PipboyArtSymbols.DollPiece(piece, tree));
        }

        [Fact]
        public void DollPiece_UnknownInstanceOrLabel_Throws()
        {
            var tree = DollTree();

            Assert.Throws<SwfFormatException>(() => PipboyArtSymbols.DollPiece(new DollPieceSource("part/x", "Page.swf", "Host", "Missing_mc", null), tree));
            Assert.Throws<SwfFormatException>(() => PipboyArtSymbols.DollPiece(new DollPieceSource("part/x", "Page.swf", "Host", null, "Missing"), tree));
            Assert.Throws<ArgumentException>(() => PipboyArtSymbols.DollPiece(new DollPieceSource("part/x", "Page.swf", "Host", null, null), tree));
        }

        [Fact]
        public void ContentBounds_CoverContentLeftOfAndAboveTheOrigin_AcrossAllFrames()
        {
            var tree = Tree(swf =>
            {
                Box(swf, 1, 0, -1526, 1145, 0);
                Box(swf, 3, 0, 0, 100, 100);
                swf.Sprite(2, t => t.Place(At(1, 1)).ShowFrame());
                swf.Root.Place(At(3, 1)).Place(At(2, 2, -396, -556)).ShowFrame()
                    .Place(new SwfPlacement(2) { Move = true, Matrix = SwfPlacement.Translate(500, -556) }).ShowFrame();
            });

            Assert.Equal(new TwipRect(-396, -2082, 1645, 100), tree.ContentBounds());
        }

        [Fact]
        public void ContentBounds_RemovedAndReplacedCharacters_AreStillCounted()
        {
            var tree = Tree(swf =>
            {
                Box(swf, 1, 0, 0, 10, 10);
                Box(swf, 2, -50, 0, 0, 10);
                swf.Root.Place(At(1, 1)).ShowFrame().Remove(1).Place(At(2, 1)).ShowFrame();
            });

            Assert.Equal(new TwipRect(-50, 0, 10, 10), tree.ContentBounds());
        }

        [Fact]
        public void ContentBounds_ReplacementWithoutAMatrix_KeepsTheDepthsMatrix()
        {
            var tree = Tree(swf =>
            {
                Box(swf, 1, 0, 0, 10, 10);
                Box(swf, 2, 0, 0, 20, 20);
                swf.Root.Place(At(1, 1, 100, 50)).ShowFrame()
                    .Place(new SwfPlacement(1) { Move = true, Character = 2 }).ShowFrame();
            });

            Assert.Equal(new TwipRect(100, 50, 120, 70), tree.ContentBounds());
        }

        [Fact]
        public void ContentBounds_CharacterOfUnknownKind_Throws()
        {
            var tree = Tree(swf => swf.Root.Place(At(9, 1)).ShowFrame());

            Assert.Throws<SwfFormatException>(() => tree.ContentBounds());
        }

        [Fact]
        public void ContentBounds_MovieThatShowsNothing_Throws()
        {
            Assert.Throws<SwfFormatException>(() => Tree(swf => swf.Root.ShowFrame()).ContentBounds());
        }

        [Fact]
        public void FrameRanges_CollapseConsecutiveFramesIntoRuns()
        {
            Assert.Equal("1-4,7,9-10", PipboyArtExporter.FrameRanges([10, 1, 2, 3, 4, 7, 9, 2]));
            Assert.Equal("5", PipboyArtExporter.FrameRanges([5]));
        }

        const string FrameWithContent =
            "<svg xmlns=\"http://www.w3.org/2000/svg\" height=\"20.0px\" width=\"10.0px\"><g transform=\"matrix(1.0, 0.0, 0.0, 1.0, 5.0, 10.0)\"><path d=\"M0 0\"/></g></svg>";

        const string BlankSvg =
            "<svg xmlns=\"http://www.w3.org/2000/svg\" height=\"20.0px\" width=\"10.0px\"><g transform=\"matrix(1.0, 0.0, 0.0, 1.0, 5.0, 10.0)\"/></svg>";

        [Fact]
        public void SymbolClip_BlankFrameKeepsItsPlace_AndTheBoundsCoverTheRest()
        {
            var clip = PipboyArtExporter.SymbolClip("icon/hotkey", "Page.swf", null, ArtColour.Mask, 30, [],
                [SvgFrame.Parse(BlankSvg), SvgFrame.Parse(FrameWithContent)]);

            Assert.Equal(2, clip.Frames.Count);
            Assert.Equal(new ArtBounds(-5, -10, 5, 10), clip.Bounds);
            Assert.DoesNotContain("<path", clip.Frames[0]);
            Assert.Contains("<path", clip.Frames[1]);
        }

        [Fact]
        public void SymbolClip_NoFrameShowsAnything_Throws()
        {
            Assert.Throws<PipboyArtException>(() =>
                PipboyArtExporter.SymbolClip("icon/x", "Page.swf", null, ArtColour.Mask, 30, [], [SvgFrame.Parse(BlankSvg)]));
        }

        [Fact]
        public void SymbolClip_KeepsTheScriptAndFrameRate()
        {
            ClipScriptStep[] script = [new(1, "stop", null)];

            var clip = PipboyArtExporter.SymbolClip("icon/x", "Page.swf", null, ArtColour.Mask, 24, script, [SvgFrame.Parse(FrameWithContent)]);

            Assert.Equal(script, clip.Script);
            Assert.Equal(24, clip.FramesPerSecond);
        }

        [Fact]
        public void Manifest_FormatProblem_IsNoneOnlyForTheFormatTheInstallerWrites()
        {
            using var dir = new TempDirectory();
            string path = Path.Combine(dir.Path, "art_manifest.json");

            Assert.NotNull(PipboyArtWriter.FormatProblem(path));
            File.WriteAllText(path, "{\"format\":2,\"clips\":[]}");
            Assert.NotNull(PipboyArtWriter.FormatProblem(path));
            File.WriteAllText(path, "not json");
            Assert.NotNull(PipboyArtWriter.FormatProblem(path));
            File.WriteAllText(path, $"{{\"format\":{PipboyArtWriter.ManifestFormat},\"clips\":[]}}");
            Assert.Null(PipboyArtWriter.FormatProblem(path));
        }
    }
}
