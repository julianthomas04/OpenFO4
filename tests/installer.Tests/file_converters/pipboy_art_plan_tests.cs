using AssetConverter;
using AssetConverter.Swf;
using Xunit;

namespace InstallerTests
{
    // The clips of an export are the catalog's symbols plus what the movies themselves state
    // (damage type count, marker classes, perk cards) and what the archive holds (condition figures, Vault Boy movies).
    public sealed class PipboyArtPlanTests
    {
        static byte[] Plain() => SyntheticArtArchive.Plain();

        static Dictionary<string, ArtMovie> Archive(int damageTypes = 6, string[]? markers = null, int bodies = 3, params string[] extra) =>
            SyntheticArtArchive.Movies(damageTypes, markers, bodies, extra);

        [Fact]
        public void Build_MarkerClips_FollowTheMapsClassListInTypeOrder()
        {
            var plan = PipboyArtPlan.Build(Archive(markers: ["FirstMarker", "SecondMarker", "ThirdMarker"]));

            Assert.Equal(["marker/0", "marker/1", "marker/2"], plan.ClipIds.Where(id => id.StartsWith("marker/", StringComparison.Ordinal)));
        }

        [Fact]
        public void Build_PerkCards_AreTheLevelUpMenusPerkClipSymbolsInClassOrder()
        {
            var plan = PipboyArtPlan.Build(Archive());

            var cards = plan.Symbols.Where(work => work.ClipId.StartsWith("perkcard/", StringComparison.Ordinal)).ToList();
            Assert.Equal(["perkcard/0", "perkcard/1"], cards.Select(card => card.ClipId));
            Assert.Equal([PipboyArtCatalog.PerkMoviePath("PerkClip0a"), PipboyArtCatalog.PerkMoviePath("PerkClip2b")], cards.Select(card => card.Illustrates));
        }

        [Fact]
        public void Build_ConditionBodies_AreTheNumberedMoviesTheArchiveHolds()
        {
            var plan = PipboyArtPlan.Build(Archive(bodies: 4));

            Assert.Equal(["condition/body_00", "condition/body_01", "condition/body_02", "condition/body_03", "condition/head"],
                plan.ClipIds.Where(id => id.StartsWith("condition/", StringComparison.Ordinal)).Order(StringComparer.Ordinal));
        }

        [Fact]
        public void Build_VaultBoyMovies_BecomeWholeMovieClips()
        {
            var plan = PipboyArtPlan.Build(Archive(extra: [@"Components\VaultBoys\Perks\Sample .swf", @"Components\Magazine perks\Mag.swf"]));

            Assert.Contains("vaultboy/components/vaultboys/perks/sample", plan.ClipIds);
            Assert.Contains("vaultboy/components/magazine perks/mag", plan.ClipIds);
        }

        [Fact]
        public void Build_LoadingIcon_IsAnEveryFrameMaskClipOfTheLoadingScreen()
        {
            var plan = PipboyArtPlan.Build(Archive());

            var icon = plan.Symbols.Single(work => work.ClipId == PipboyArtCatalog.LoadingIconClipId);
            Assert.Equal(PipboyArtCatalog.LoadingScreen, icon.Movie);
            Assert.Equal(ArtColour.Mask, icon.Colour);
        }

        [Fact]
        public void Build_LoadingScreenMissingFromTheArchive_NamesIt()
        {
            var movies = Archive();
            movies.Remove(PipboyArtCatalog.LoadingScreen);

            var failure = Assert.Throws<PipboyArtException>(() => PipboyArtPlan.Build(movies));

            Assert.Contains(PipboyArtCatalog.LoadingScreen, failure.Message);
        }

        [Fact]
        public void ResolveAll_LoadingIcon_IsTheAnimationInsideTheLogoInstanceWithEveryFrame()
        {
            var movie = new ArtMovie(PipboyArtCatalog.LoadingScreen, new SwfMovieBuilder()
                .Shape(1, new TwipRect(0, 0, 20, 20), new SwfColor(255, 255, 255, 255))
                .Sprite(14, t => t.Place(new SwfPlacement(1) { Character = 1 }).ShowFrame().ShowFrame().ShowFrame())
                .Sprite(15, t => t.Place(new SwfPlacement(1) { Character = 14 }).ShowFrame())
                .Sprite(20, t => t.Place2(new SwfPlacement(3) { Character = 15, Name = "VaultTecLogo_mc" }).ShowFrame())
                .SymbolClass((20, PipboyArtCatalog.LoadingScreenMenuClass)).Build());
            var movies = new Dictionary<string, ArtMovie> { [PipboyArtCatalog.LoadingScreen] = movie };
            var plan = PipboyArtPlan.Build(Archive());
            var icon = plan.Symbols.Single(work => work.ClipId == PipboyArtCatalog.LoadingIconClipId);

            var resolved = PipboyArtPlan.ResolveAll(movies, new PipboyArtPlan([icon], []));

            Assert.Equal(14, resolved.Single().Symbol.CharacterId);
            Assert.Equal([1, 2, 3], resolved.Single().Symbol.Frames);
        }

        [Fact]
        public void Build_ClipIds_AreUnique()
        {
            var ids = PipboyArtPlan.Build(Archive()).ClipIds;

            Assert.Equal(ids.Count, ids.Distinct().Count());
        }

        [Fact]
        public void Build_DamageClips_FollowTheStatedTypeCount()
        {
            var plan = PipboyArtPlan.Build(Archive(damageTypes: 6));

            var frames = plan.Symbols.Where(work => work.ClipId.StartsWith("icon/damage_", StringComparison.Ordinal)).ToList();
            Assert.Equal(PipboyArtCatalog.NativeDamageTypes.Count * 2, frames.Count);
        }

        [Fact]
        public void Build_DamageTypeCountTheStandInDoesNotCover_Throws()
        {
            var failure = Assert.Throws<PipboyArtException>(() => PipboyArtPlan.Build(Archive(damageTypes: 7)));

            Assert.Contains("7 damage types", failure.Message);
        }

        [Fact]
        public void Build_NoDamageTypeCount_Throws()
        {
            var movies = Archive();
            movies[PipboyArtCatalog.InventoryPage] = new ArtMovie(PipboyArtCatalog.InventoryPage, Plain());

            var failure = Assert.Throws<PipboyArtException>(() => PipboyArtPlan.Build(movies));

            Assert.Contains(PipboyArtCatalog.DamageTypeCountConstant, failure.Message);
        }

        [Fact]
        public void Build_NoMarkerList_Throws()
        {
            var movies = Archive();
            movies[PipboyArtCatalog.MapPage] = new ArtMovie(PipboyArtCatalog.MapPage, Plain());

            var failure = Assert.Throws<PipboyArtException>(() => PipboyArtPlan.Build(movies));

            Assert.Contains(PipboyArtCatalog.MarkerListMember, failure.Message);
        }

        [Fact]
        public void Build_MovieMissingFromTheArchive_NamesIt()
        {
            var movies = Archive();
            movies.Remove(PipboyArtCatalog.Shell);

            var failure = Assert.Throws<PipboyArtException>(() => PipboyArtPlan.Build(movies));

            Assert.Contains(PipboyArtCatalog.Shell, failure.Message);
        }

        [Fact]
        public void Build_PerkMenuWithoutCards_Throws()
        {
            var movies = Archive();
            movies[PipboyArtCatalog.LevelUpMenu] = new ArtMovie(PipboyArtCatalog.LevelUpMenu, Plain());

            Assert.Throws<PipboyArtException>(() => PipboyArtPlan.Build(movies));
        }

        [Fact]
        public void Build_MalformedMovie_NamesTheMovie()
        {
            var movies = Archive();
            movies[PipboyArtCatalog.InventoryPage] = new ArtMovie(PipboyArtCatalog.InventoryPage, [1, 2, 3]);

            var failure = Assert.Throws<PipboyArtException>(() => PipboyArtPlan.Build(movies));

            Assert.Contains(PipboyArtCatalog.InventoryPage, failure.Message);
        }

        [Fact]
        public void ResolveAll_SymbolNotInItsMovie_NamesTheClipAndTheMovie()
        {
            var movies = new Dictionary<string, ArtMovie> { ["Page.swf"] = new("Page.swf", Plain()) };
            var work = new SymbolWork("icon/x", "Page.swf", null, ArtColour.Mask, tree => new ResolvedSymbol(tree.CharacterOfClass("Missing"), [1]));

            var failure = Assert.Throws<PipboyArtException>(() => PipboyArtPlan.ResolveAll(movies, new PipboyArtPlan([work], [])));

            Assert.Contains("icon/x", failure.Message);
            Assert.Contains("Page.swf", failure.Message);
        }

        [Fact]
        public void ResolveAll_FindsEverySymbolOfAMovie()
        {
            var movie = new ArtMovie("Page.swf", new SwfMovieBuilder().Shape(1, new TwipRect(0, 0, 20, 20), new SwfColor(0, 0, 0, 255)).SymbolClass((1, "Glyph")).Build());
            var movies = new Dictionary<string, ArtMovie> { ["Page.swf"] = movie };
            var work = new SymbolWork("icon/x", "Page.swf", null, ArtColour.Mask, tree => new ResolvedSymbol(tree.CharacterOfClass("Glyph"), [1]));

            var resolved = PipboyArtPlan.ResolveAll(movies, new PipboyArtPlan([work], []));

            Assert.Equal(1, resolved.Single().Symbol.CharacterId);
        }

        [Fact]
        public void ConditionMovies_BodiesWithAGap_Throw()
        {
            Assert.Throws<PipboyArtException>(() => PipboyArtCatalog.ConditionMovies(
                [PipboyArtCatalog.ConditionHeadMovie, @"Components\ConditionClips\Condition_Body_0.swf", @"Components\ConditionClips\Condition_Body_2.swf"]));
        }

        [Fact]
        public void ConditionMovies_NoHeadOrNoBody_Throw()
        {
            Assert.Throws<PipboyArtException>(() => PipboyArtCatalog.ConditionMovies([@"Components\ConditionClips\Condition_Body_0.swf"]));
            Assert.Throws<PipboyArtException>(() => PipboyArtCatalog.ConditionMovies([PipboyArtCatalog.ConditionHeadMovie]));
        }

        [Fact]
        public void NativeDamageTypes_MatchingCount_IsAccepted()
        {
            PipboyArtCatalog.RequireMatchingTypeCount(PipboyArtCatalog.NativeDamageTypes.Count);
        }
    }
}
