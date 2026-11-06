namespace AssetConverter.Pipeline
{
    /// <summary>What an install reads from the install it runs over and what it makes of that: the record, the resulting plan and a warning when the record could not be read.</summary>
    internal sealed record ReconversionDecision(InstalledAssets? Installed, ReconversionPlan Plan, string? Warning)
    {
        /// <summary>
        /// Decides from the ownership marker at <paramref name="markerPath"/>. A marker that cannot be read vouches for nothing, so every group is
        /// converted; the warning says so, and the install fails when it then cannot write the marker either.
        /// </summary>
        public static ReconversionDecision Read(string markerPath, IReadOnlyDictionary<AssetGroup, int> current)
        {
            InstalledAssets? installed;
            try
            {
                installed = AssetVersionMarker.Read(markerPath);
            }
            catch (Exception ex) when (ex is IOException or UnauthorizedAccessException)
            {
                return new(null, ReconversionPlan.ReconvertAll, $"WARNING: the install record '{markerPath}' cannot be read ({ex.Message}); every asset group is converted.");
            }
            return new(installed, ReconversionPlan.Decide(installed, current), null);
        }

        /// <summary>The lines that say which groups the install keeps and which it converts, with the revisions behind each choice.</summary>
        public IReadOnlyList<string> Describe(IReadOnlyDictionary<AssetGroup, int> current)
        {
            var lines = new List<string>();
            if (Warning is not null)
                lines.Add(Warning);
            lines.Add(Installed is null
                ? "No readable record of an earlier install: every asset group is converted."
                : $"Asset groups of the install being updated (recorded by installer {Installed.Version}):");
            lines.AddRange(AssetGroups.All.Select(group => $"  {AssetGroups.Id(group)}: {Verdict(group, current[group])}"));
            return lines;
        }

        private string Verdict(AssetGroup group, int revision)
        {
            if (!Plan.Reconverts(group))
                return $"kept (revision {revision})";
            if (Installed is null)
                return $"converted (revision {revision})";
            if (!Installed.Revisions.TryGetValue(group, out int installed))
                return $"converted (not recorded, revision {revision})";
            return installed != revision
                ? $"converted (revision {installed} -> {revision})"
                : $"converted (revision {revision}, built from a group that is converted)";
        }
    }
}
