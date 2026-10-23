using AssetConverter;
using AssetConverter.Swf;
using Xunit;

namespace InstallerTests
{
    public sealed class SwfTimelinePlayerTests
    {
        private static SwfPlayedTimeline Play(Action<SwfTimelineWriter> timeline)
        {
            var swf = new SwfMovieBuilder();
            timeline(swf.Root);
            return SwfTimelinePlayer.Play(swf.Read().Root, "the test timeline");
        }

        private static SwfPlacement At(int depth, int character, double x = 0, double y = 0) =>
            new(depth) { Character = character, Matrix = SwfPlacement.Translate(x, y) };

        [Fact]
        public void Play_EachFrameIsTheDisplayListAsItStandsThere_OrderedByDepth()
        {
            var played = Play(t => t.Place(At(5, 1)).Place(At(2, 2)).ShowFrame().Place(At(3, 3)).ShowFrame());

            Assert.Equal([2, 5], played.Frames[0].Select(o => o.Depth));
            Assert.Equal([2, 3, 5], played.Frames[1].Select(o => o.Depth));
        }

        [Fact]
        public void Play_TagsAfterTheLastFrame_NeverAppear()
        {
            Assert.Throws<SwfFormatException>(() => Play(t => t.Place(At(1, 1)).ShowFrame().Place(At(2, 2))));
        }

        [Fact]
        public void Play_ModificationWithoutACharacter_ChangesTheMatrixAndKeepsEverythingElse()
        {
            var played = Play(t => t
                .Place(At(1, 7) with { Name = "a", Color = SwfPlacement.Alpha(0.5), Visible = false }).ShowFrame()
                .Place(new SwfPlacement(1) { Move = true, Matrix = SwfPlacement.Translate(40, 60) }).ShowFrame());

            SwfDisplayObject before = played.Frames[0][0], after = played.Frames[1][0];
            Assert.Equal(SwfPlacement.Translate(40, 60), after.Matrix);
            Assert.Equal(before with { Matrix = after.Matrix }, after);
        }

        [Fact]
        public void Play_ModificationOfOnlyTheColour_KeepsTheMatrix()
        {
            var played = Play(t => t.Place(At(1, 7, 100, 200)).ShowFrame().Place(new SwfPlacement(1) { Move = true, Color = SwfPlacement.Alpha(0.25) }).ShowFrame());

            Assert.Equal(SwfPlacement.Translate(100, 200), played.Frames[1][0].Matrix);
            Assert.Equal(0.25, played.Frames[1][0].Color.AMult);
        }

        [Fact]
        public void Play_MoveWithACharacter_ReplacesTheCharacterKeepsTheTransformAndStartsANewLife()
        {
            var played = Play(t => t.Place(At(1, 7, 100, 200) with { Name = "a" }).ShowFrame().Place(new SwfPlacement(1) { Move = true, Character = 8 }).ShowFrame());

            SwfDisplayObject before = played.Frames[0][0], after = played.Frames[1][0];
            Assert.Equal(8, after.CharacterId);
            Assert.Equal(before.Matrix, after.Matrix);
            Assert.Equal("a", after.Name);
            Assert.NotEqual(before.Life, after.Life);
        }

        [Fact]
        public void Play_ModificationWithoutACharacter_KeepsTheLife()
        {
            var played = Play(t => t.Place(At(1, 7)).ShowFrame().Place(new SwfPlacement(1) { Move = true, Matrix = SwfPlacement.Translate(1, 1) }).ShowFrame());

            Assert.Equal(played.Frames[0][0].Life, played.Frames[1][0].Life);
        }

        [Fact]
        public void Play_NewCharacterWithoutAMatrixOrColour_StartsAtTheIdentity()
        {
            var placed = Assert.Single(Play(t => t.Place(new SwfPlacement(1) { Character = 7 }).ShowFrame()).Frames[0]);

            Assert.Equal(SwfMatrix.Identity, placed.Matrix);
            Assert.Equal(ColorTransform.Identity, placed.Color);
            Assert.Equal(SwfBlendMode.Normal, placed.BlendMode);
            Assert.True(placed.Visible);
            Assert.Empty(placed.Filters);
        }

        [Fact]
        public void Play_Removal_ClearsTheDepthFromThenOn()
        {
            var played = Play(t => t.Place(At(1, 7)).ShowFrame().Remove(1).ShowFrame().ShowFrame());

            Assert.Equal([1, 0, 0], played.Frames.Select(f => f.Count));
        }

        [Fact]
        public void Play_HiddenPlacementStaysHiddenUntilAModificationShowsIt()
        {
            var played = Play(t => t
                .Place(At(1, 7) with { Visible = false }).ShowFrame()
                .Place(new SwfPlacement(1) { Move = true, Matrix = SwfPlacement.Translate(5, 5) }).ShowFrame()
                .Place(new SwfPlacement(1) { Move = true, Visible = true }).ShowFrame());

            Assert.Equal([false, false, true], played.Frames.Select(f => f[0].Visible));
        }

        [Fact]
        public void Play_EffectsOfAPlacement_AreKept()
        {
            var placed = Assert.Single(Play(t => t.Place(At(1, 7) with
            {
                Filters = SwfPlacement.BlurFilter(4, 2, 3),
                BlendMode = 8,
                CacheAsBitmap = true,
                Ratio = 12,
                ClipDepth = 4,
                ClassName = "Some.Class",
            }).ShowFrame()).Frames[0]);

            Assert.Equal(new BlurFilter(4, 2, 3), Assert.Single(placed.Filters));
            Assert.Equal(SwfBlendMode.Add, placed.BlendMode);
            Assert.True(placed.CacheAsBitmap);
            Assert.Equal(12, placed.Ratio);
            Assert.True(placed.IsMask);
            Assert.Equal("Some.Class", placed.ClassName);
        }

        [Theory]
        [InlineData(0)]
        [InlineData(1)]
        public void Play_BlendModeZeroAndOne_AreBothNormal(int blendMode)
        {
            Assert.Equal(SwfBlendMode.Normal, Play(t => t.Place(At(1, 7) with { BlendMode = blendMode }).ShowFrame()).Frames[0][0].BlendMode);
        }

        [Fact]
        public void Play_Labels_NameTheirOneBasedFrame()
        {
            var played = Play(t => t.Label("start").ShowFrame().ShowFrame().Label("loop").ShowFrame());

            Assert.Equal(new Dictionary<string, int> { ["start"] = 1, ["loop"] = 3 }, played.Labels);
        }

        [Fact]
        public void Play_LabelUsedTwice_Throws()
        {
            Assert.Throws<SwfFormatException>(() => Play(t => t.Label("a").ShowFrame().Label("a").ShowFrame()));
        }

        [Fact]
        public void Play_ModificationOfAnEmptyDepth_Throws()
        {
            Assert.Throws<SwfFormatException>(() => Play(t => t.Place(new SwfPlacement(1) { Move = true, Matrix = SwfMatrix.Identity }).ShowFrame()));
        }

        [Fact]
        public void Play_CharacterOnAnOccupiedDepthWithoutAMove_Throws()
        {
            Assert.Throws<SwfFormatException>(() => Play(t => t.Place(At(1, 7)).Place(At(1, 8)).ShowFrame()));
        }

        [Fact]
        public void Play_PlaceObject2_ReadsTheSameAsPlaceObject3()
        {
            var placement = At(1, 7, 30, 40) with { Name = "n", Color = SwfPlacement.Alpha(0.5), ClipDepth = 3 };

            SwfDisplayObject version3 = Play(t => t.Place(placement).ShowFrame()).Frames[0][0];
            SwfDisplayObject version2 = Play(t => t.Place2(placement).ShowFrame()).Frames[0][0];

            Assert.True(version3.PlacementEquals(version2));
            Assert.Equal(version3.CharacterId, version2.CharacterId);
        }

        [Fact]
        public void Play_ImageFlagWithACharacter_ReadsNoClassName()
        {
            SwfDisplayObject placed = Play(t => t.Place(At(1, 7, 30, 40) with { IsImage = true }).ShowFrame()).Frames[0][0];

            Assert.Equal(7, placed.CharacterId);
            Assert.Equal(SwfPlacement.Translate(30, 40), placed.Matrix);
            Assert.Null(placed.ClassName);
        }

        [Fact]
        public void Play_RotationAndSkewTerms_AreReadIntoTheirMatrixSlots()
        {
            var matrix = new SwfMatrix(0.5, 1.25, -2, 3, -100, 80);

            Assert.Equal(matrix, Play(t => t.Place(new SwfPlacement(1) { Character = 7, Matrix = matrix }).ShowFrame()).Frames[0][0].Matrix);
        }

        [Fact]
        public void Play_ColourTransformTerms_AreFractionsAndChannelOffsets()
        {
            var color = new ColorTransform(0.5, 2, 1, 0.25, 10, -20, 0, 5);

            Assert.Equal(color, Play(t => t.Place(At(1, 7) with { Color = color }).ShowFrame()).Frames[0][0].Color);
        }
    }
}
