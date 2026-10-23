using Mutagen.Bethesda.Fallout4;
using Mutagen.Bethesda.Plugins;

namespace AssetConverter
{
    /// <summary>One perk-chart cell: the first rank of a perk and the SPECIAL value that unlocks it.</summary>
    internal sealed record PerkChartEntry(FormKey Perk, string Special, int RequiredSpecial);

    /// <summary>
    /// Derives the perk chart from the PERK records: a chart perk is the first rank of a rank chain (see
    /// <see cref="PerkRankChains"/>) that is playable, non-hidden and non-trait, and whose conditions require exactly one
    /// SPECIAL attribute to be at least some value. That attribute is the chart column and the value the chart row. Perks
    /// that raise a SPECIAL attribute carry a "below" condition and are not chart perks.
    /// </summary>
    internal static class PerkChartBuilder
    {
        /// <summary>The chart has one column per SPECIAL attribute.</summary>
        public static int ColumnCount => SpecialAttributes.Keys.Count;

        /// <summary>The chart has as many rows as the highest SPECIAL value a first-rank perk requires.</summary>
        public static int RowCount(IReadOnlyList<PerkChartEntry> chart) => chart.Select(entry => entry.RequiredSpecial).DefaultIfEmpty(0).Max();

        public static IReadOnlyList<PerkChartEntry> Build(
            IReadOnlyCollection<IPerkGetter> perks,
            IEnumerable<IActorValueInformationGetter> actorValues,
            PerkRankChains chains)
        {
            var specialByKey = SpecialAttributes.KeysByRecord(actorValues);
            var chartPerks = perks
                .Where(perk => chains.FirstRankOf[perk.FormKey] == perk.FormKey)
                .Where(perk => perk.Playable && !perk.Hidden && !perk.Trait)
                .Select(perk => (Perk: perk, Requirement: SpecialRequirement(perk, specialByKey)))
                .Where(candidate => candidate.Requirement is not null)
                .Select(candidate => new PerkChartEntry(candidate.Perk.FormKey, candidate.Requirement!.Value.Special, candidate.Requirement.Value.Value))
                .Where(entry => entry.RequiredSpecial >= 1);
            return chartPerks.OrderBy(entry => entry.Special, StringComparer.Ordinal).ThenBy(entry => entry.RequiredSpecial).ThenBy(entry => entry.Perk.ID).ToList();
        }

        /// <summary>Distinct (SPECIAL, row) cells covered, so sex-specific variants sharing a cell count once.</summary>
        public static int CoveredCells(IEnumerable<PerkChartEntry> entries) =>
            entries.Select(entry => (entry.Special, entry.RequiredSpecial)).Distinct().Count();

        /// <summary>The base game's chart fills every cell of its columns and rows; anything else means the extraction rule no longer fits the data.</summary>
        public static void RequireCompleteChart(IReadOnlyList<PerkChartEntry> chart)
        {
            if (chart.Count == 0)
                throw new InvalidDataException("The perk chart is empty; the chart extraction no longer matches the game data.");
            int cells = CoveredCells(chart);
            int expectedCells = ColumnCount * RowCount(chart);
            if (cells != expectedCells)
                throw new InvalidDataException($"The perk chart fills {cells} of {expectedCells} cells ({chart.Count} first-rank perks); the chart extraction no longer matches the game data.");
        }

        static (string Special, int Value)? SpecialRequirement(IPerkGetter perk, Dictionary<FormKey, string> specialByKey)
        {
            var requirements = new List<(string Special, int Value)>();
            foreach (var condition in perk.Conditions)
            {
                if (condition is not IConditionFloatGetter { CompareOperator: CompareOperator.GreaterThanOrEqualTo } floatCondition) continue;
                if (floatCondition.Data is not IFunctionConditionDataGetter { Function: Condition.Function.GetBaseValue } function) continue;
                if (!specialByKey.TryGetValue(function.ParameterOneRecord.FormKey, out var special)) continue;
                requirements.Add((special, (int)Math.Round(floatCondition.ComparisonValue)));
            }
            return requirements.Count == 1 ? requirements[0] : null;
        }
    }
}
