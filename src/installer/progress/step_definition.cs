using AssetConverter.Pipeline;

namespace AssetConverter.Progress
{
    /// <summary>
    /// One declared unit of installation work. <paramref name="Weight"/> is its share of the overall progress bar relative to the others.
    /// Units that share a <paramref name="Group"/> are shown as one step of that name, running while any of them runs; the others are shown under their own name.
    /// <paramref name="KnownTotal"/> is the total of a step whose work is fixed in advance; it counts as final before the step begins, and the step must begin with it.
    /// <see cref="Groups"/> are the asset groups the step does work for; the step runs when any of them is converted, and a step with none always runs.
    /// </summary>
    public sealed record StepDefinition(string Name, string? Unit = null, double Weight = 1.0, string? Group = null, long? KnownTotal = null)
    {
        public string VisibleName => Group ?? Name;

        internal IReadOnlyList<AssetGroup> Groups { get; init; } = [];

        internal bool RunsFor(ReconversionPlan plan) => Groups.Count == 0 || Groups.Any(plan.Reconverts);
    }
}
