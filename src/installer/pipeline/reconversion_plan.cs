namespace AssetConverter.Pipeline
{
    /// <summary>
    /// Which asset groups an install keeps from the install it runs over and which it converts (again): the groups that are missing from
    /// that install or were converted at another revision, plus every group built from one of those.
    /// </summary>
    internal sealed class ReconversionPlan
    {
        private readonly IReadOnlySet<AssetGroup> _reconverted;

        private ReconversionPlan(IReadOnlySet<AssetGroup> reconverted) => _reconverted = reconverted;

        /// <summary>The plan of an install with nothing to keep: a fresh folder or a marker that vouches for no group.</summary>
        public static ReconversionPlan ReconvertAll { get; } = new(new HashSet<AssetGroup>(AssetGroups.All));

        public IReadOnlySet<AssetGroup> Reconverted => _reconverted;

        public IReadOnlySet<AssetGroup> Kept => new HashSet<AssetGroup>(AssetGroups.All.Where(group => !Reconverts(group)));

        public bool Reconverts(AssetGroup group) => _reconverted.Contains(group);

        /// <param name="installed">What the existing install recorded; null when it recorded nothing readable.</param>
        /// <param name="current">The revision of every group as this installer converts it.</param>
        public static ReconversionPlan Decide(InstalledAssets? installed, IReadOnlyDictionary<AssetGroup, int> current)
        {
            if (installed is null)
                return ReconvertAll;

            var changed = AssetGroups.All.Where(group => !installed.Revisions.TryGetValue(group, out int revision) || revision != current[group]);
            return new ReconversionPlan(AssetGroups.WithDependents(changed));
        }
    }
}
