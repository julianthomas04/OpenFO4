namespace AssetConverter.Progress
{
    /// <summary>
    /// One declared unit of installation work. <paramref name="Weight"/> is its share of the overall progress bar relative to the others.
    /// Units that share a <paramref name="Group"/> are shown as one step of that name, running while any of them runs; the others are shown under their own name.
    /// <paramref name="KnownTotal"/> is the total of a step whose work is fixed in advance; it counts as final before the step begins, and the step must begin with it.
    /// </summary>
    public sealed record StepDefinition(string Name, string? Unit = null, double Weight = 1.0, string? Group = null, long? KnownTotal = null)
    {
        public string VisibleName => Group ?? Name;
    }
}
