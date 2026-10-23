using AssetConverter;
using Mutagen.Bethesda.Fallout4;
using Mutagen.Bethesda.Plugins;
using Xunit;

namespace InstallerTests
{
    // Rank chain heads. The head is stored per perk so no consumer repeats the rule.
    public class PerkRankChainsTests
    {
        static readonly ModKey Mod = ModKey.FromNameAndExtension("Chains.esp");

        static Perk Rank(uint id, byte level, uint? next = null)
        {
            var perk = new Perk(new FormKey(Mod, id), Fallout4Release.Fallout4) { Level = level };
            if (next is not null) perk.NextPerk.FormKey = new FormKey(Mod, next.Value);
            return perk;
        }

        static FormKey Key(uint id) => new(Mod, id);

        [Fact]
        public void ClosedChain_HeadIsTheLowestLevelRankWhateverTheListingOrder()
        {
            var chains = PerkRankChains.Build([Rank(3, 30, next: 1), Rank(1, 1, next: 2), Rank(2, 15, next: 3)]);

            Assert.All(new uint[] { 1, 2, 3 }, id => Assert.Equal(Key(1), chains.FirstRankOf[Key(id)]));
        }

        [Fact]
        public void ClosedChain_WithTiedLevels_PicksTheLowestFormIdAndIsDeterministic()
        {
            var forward = PerkRankChains.Build([Rank(5, 10, next: 6), Rank(6, 10, next: 7), Rank(7, 10, next: 5)]);
            var reversed = PerkRankChains.Build([Rank(7, 10, next: 5), Rank(6, 10, next: 7), Rank(5, 10, next: 6)]);

            Assert.Equal(Key(5), forward.FirstRankOf[Key(7)]);
            Assert.Equal(forward.FirstRankOf, reversed.FirstRankOf);
        }

        [Fact]
        public void OpenChain_HeadIsTheRankNothingLinksToEvenWhenItIsNotTheLowestLevel()
        {
            var chains = PerkRankChains.Build([Rank(10, 20, next: 11), Rank(11, 2)]);

            Assert.Equal(Key(10), chains.FirstRankOf[Key(10)]);
            Assert.Equal(Key(10), chains.FirstRankOf[Key(11)]);
        }

        [Fact]
        public void PerkWithoutLinks_IsItsOwnHead()
        {
            var chains = PerkRankChains.Build([Rank(20, 1), Rank(21, 1)]);

            Assert.Equal(Key(20), chains.FirstRankOf[Key(20)]);
            Assert.Equal(Key(21), chains.FirstRankOf[Key(21)]);
            Assert.Empty(chains.Warnings);
        }

        [Fact]
        public void LinkToAPerkOutsideTheSet_IsIgnored()
        {
            var chains = PerkRankChains.Build([Rank(30, 5, next: 999)]);

            Assert.Equal(Key(30), chains.FirstRankOf[Key(30)]);
        }

        [Fact]
        public void TwoRanksLinkingToTheSameNext_AreReportedAndFormOneChain()
        {
            var chains = PerkRankChains.Build([Rank(40, 1, next: 42), Rank(41, 5, next: 42), Rank(42, 9)]);

            var warning = Assert.Single(chains.Warnings);
            Assert.Contains(Key(42).ToString(), warning);
            Assert.Equal(Key(40), chains.FirstRankOf[Key(41)]);
        }

        [Fact]
        public void SeparateChains_KeepSeparateHeads()
        {
            var chains = PerkRankChains.Build([Rank(1, 1, next: 2), Rank(2, 9), Rank(3, 1, next: 4), Rank(4, 9)]);

            Assert.Equal(Key(1), chains.FirstRankOf[Key(2)]);
            Assert.Equal(Key(3), chains.FirstRankOf[Key(4)]);
        }
    }
}
