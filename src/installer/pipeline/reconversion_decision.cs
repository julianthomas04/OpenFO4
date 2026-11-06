using AssetConverter.UiLayout;

namespace AssetConverter.Pipeline
{
    /// <summary>
    /// What an install reads from the install it runs over and what it makes of that: the record, the inputs the game has now, the resulting
    /// plan and a warning when the record or the inputs could not be read.
    /// </summary>
    /// <param name="Inputs">The game inputs of this install; null when they could not be determined.</param>
    /// <param name="Drift">Why the record's groups cannot be kept whatever their revisions (another language, other game files), or null.</param>
    internal sealed record ReconversionDecision(InstalledAssets? Installed, GameInputs? Inputs, string? Drift, ReconversionPlan Plan, string? Warning)
    {
        /// <summary>Decides for the install at <paramref name="paths"/>; inputs that cannot be read convert everything and the warning says so.</summary>
        public static ReconversionDecision ForInstall(InstallLocations paths, IReadOnlyDictionary<AssetGroup, int> current)
        {
            GameInputs? inputs = null;
            string? warning = null;
            try
            {
                inputs = GameInputs.Read(paths);
            }
            catch (Exception ex) when (ex is ConversionStepException or IOException or UnauthorizedAccessException)
            {
                warning = $"WARNING: the game language and files could not be determined ({ex.Message}); every asset group is converted.";
            }

            var decision = Read(paths.OwnershipMarker, current, inputs);
            return warning is null ? decision : decision with { Warning = warning };
        }

        /// <summary>
        /// Decides from the ownership marker at <paramref name="markerPath"/>. A marker that cannot be read vouches for nothing, so every group is
        /// converted; the warning says so, and the install fails when it then cannot write the marker either. A marker whose game inputs are not
        /// <paramref name="inputs"/> (or that records none) vouches for nothing either.
        /// </summary>
        public static ReconversionDecision Read(string markerPath, IReadOnlyDictionary<AssetGroup, int> current, GameInputs? inputs)
        {
            InstalledAssets? installed;
            try
            {
                installed = AssetVersionMarker.Read(markerPath);
            }
            catch (Exception ex) when (ex is IOException or UnauthorizedAccessException)
            {
                return new(null, inputs, null, ReconversionPlan.ReconvertAll, $"WARNING: the install record '{markerPath}' cannot be read ({ex.Message}); every asset group is converted.");
            }

            string? drift = installed is null ? null : DriftOf(installed, inputs);
            return new(installed, inputs, drift, ReconversionPlan.Decide(drift is null ? installed : null, current), null);
        }

        /// <returns>Why the groups <paramref name="installed"/> records were converted from other inputs than <paramref name="inputs"/>, or null when they were not.</returns>
        public static string? DriftOf(InstalledAssets installed, GameInputs? inputs)
        {
            if (inputs is null)
                return "the game inputs of this install are unknown";
            if (installed.Inputs is null)
                return "the record names no game language and files";
            if (installed.Inputs.Language != inputs.Language)
                return $"the game language changed from '{installed.Inputs.Language}' to '{inputs.Language}'";
            return installed.Inputs.Files == inputs.Files ? null : "the game files changed";
        }

        /// <summary>The lines that say which groups the install keeps and which it converts, with the revisions behind each choice.</summary>
        public IReadOnlyList<string> Describe(IReadOnlyDictionary<AssetGroup, int> current)
        {
            var lines = new List<string>();
            if (Warning is not null)
                lines.Add(Warning);
            if (Installed is null)
                lines.Add("No readable record of an earlier install: every asset group is converted.");
            else if (Drift is not null)
                lines.Add($"The install being updated (recorded by installer {Installed.Version}) cannot be kept: {Drift}. Every asset group is converted.");
            else
                lines.Add($"Asset groups of the install being updated (recorded by installer {Installed.Version}):");
            lines.AddRange(AssetGroups.All.Select(group => $"  {AssetGroups.Id(group)}: {Verdict(group, current[group])}"));
            return lines;
        }

        private string Verdict(AssetGroup group, int revision)
        {
            if (!Plan.Reconverts(group))
                return $"kept (revision {revision})";
            if (Installed is null || Drift is not null)
                return $"converted (revision {revision})";
            if (!Installed.Revisions.TryGetValue(group, out int installed))
                return $"converted (not recorded, revision {revision})";
            return installed != revision
                ? $"converted (revision {installed} -> {revision})"
                : $"converted (revision {revision}, built from a group that is converted)";
        }
    }
}
