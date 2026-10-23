using AssetConverter;
using Mutagen.Bethesda.Plugins;
using Xunit;

namespace InstallerTests
{
    // The all-or-nothing rule for the base game's chart.
    public class PerkChartBuilderTests
    {
        static readonly string[] Specials = ["strength", "perception", "endurance", "charisma", "intelligence", "agility", "luck"];
        const int Rows = 10;
        static readonly ModKey Mod = ModKey.FromNameAndExtension("Fallout4.esm");

        static List<PerkChartEntry> FullChart() =>
            Specials.SelectMany((special, column) => Enumerable.Range(1, Rows)
                .Select(row => new PerkChartEntry(new FormKey(Mod, (uint)(column * 100 + row)), special, row))).ToList();

        [Fact]
        public void CompleteChart_IsAccepted()
        {
            PerkChartBuilder.RequireCompleteChart(FullChart());
        }

        [Fact]
        public void ChartRows_AreTheHighestRequirementTheDataHolds()
        {
            var chart = Specials.SelectMany((special, column) => Enumerable.Range(1, 12)
                .Select(row => new PerkChartEntry(new FormKey(Mod, (uint)(column * 100 + row)), special, row))).ToList();

            Assert.Equal(12, PerkChartBuilder.RowCount(chart));
            Assert.Equal(Specials.Length, PerkChartBuilder.ColumnCount);
            PerkChartBuilder.RequireCompleteChart(chart);
        }

        [Fact]
        public void SexSpecificVariantsSharingACell_CountOnceAndDoNotBreakACompleteChart()
        {
            var chart = FullChart();
            chart.Add(new PerkChartEntry(new FormKey(Mod, 9999), "strength", 1));

            Assert.Equal(70, PerkChartBuilder.CoveredCells(chart));
            PerkChartBuilder.RequireCompleteChart(chart);
        }

        [Fact]
        public void ChartMissingACell_IsAHardError()
        {
            var chart = FullChart();
            chart.RemoveAt(0);

            var error = Assert.Throws<InvalidDataException>(() => PerkChartBuilder.RequireCompleteChart(chart));
            Assert.Contains("69 of 70", error.Message);
        }

        [Fact]
        public void EmptyChart_IsAHardError()
        {
            Assert.Throws<InvalidDataException>(() => PerkChartBuilder.RequireCompleteChart([]));
        }

        [Fact]
        public void ChartWithTooManyCells_IsAHardError()
        {
            var chart = FullChart();
            chart.Add(new PerkChartEntry(new FormKey(Mod, 9999), "strength", 11));

            Assert.Throws<InvalidDataException>(() => PerkChartBuilder.RequireCompleteChart(chart));
        }
    }
}
