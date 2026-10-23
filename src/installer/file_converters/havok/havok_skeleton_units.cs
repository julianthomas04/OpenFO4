namespace AssetConverter.Havok
{
    /// <summary>
    /// One skeleton plus the clips bound to it. All paths are absolute: the Havok skeleton and clip sources live in staging,
    /// the outputs and the NIF-converted render skeleton live under the converted meshes folder.
    /// </summary>
    public sealed record HavokSkeletonUnit(string Label, string SkeletonHkxPath, string AnimationsDir, string OutputDir, string RenderSkeletonPath);

    /// <summary>
    /// Finds the units to convert: every folder with CharacterAssets/skeleton.hkx, plus a small explicit table for creatures
    /// whose clips are bound to a skeleton stored elsewhere. Table entries are paths relative to meshes/Actors.
    /// </summary>
    public static class HavokSkeletonUnits
    {
        private sealed record Override(string Label, string SkeletonHkx, string AnimationsFolder, string OutputFolder, string RenderSkeleton);

        /// <summary>
        /// Turret clips sit in one Animations subfolder per variant, each next to its own skeleton (the clips' track counts equal
        /// the bone counts of these skeletons). ViciousDog ships no skeleton: its clips bind Dogmeat_Root, the Dogmeat skeleton.
        /// </summary>
        private static readonly IReadOnlyList<Override> Overrides = new[]
        {
            new Override("Turret/Mounted", "Turret/CharacterAssets/TurretMountedSkeleton.hkx", "Turret/Animations/Mounted",
                "Turret/CharacterAssets/Mounted", "Turret/CharacterAssets/TurretMountedSkeleton.glb"),
            new Override("Turret/Standing", "Turret/CharacterAssets/TurretStandingSkeleton.hkx", "Turret/Animations/Standing",
                "Turret/CharacterAssets/Standing", "Turret/CharacterAssets/TurretStandingSkeleton.glb"),
            new Override("Turret/Workshop", "Turret/CharacterAssets/SkeletonTurretWorkshop.hkx", "Turret/Animations/Workshop",
                "Turret/CharacterAssets/Workshop", "Turret/CharacterAssets/SkeletonTurretWorkshop.glb"),
            new Override("ViciousDog", "Dogmeat/CharacterAssets/skeleton.hkx", "ViciousDog/Animations",
                "ViciousDog/CharacterAssets", "Dogmeat/CharacterAssets/skeleton.glb"),
        };

        /// <summary>
        /// The staged NIF the unit's render skeleton GLB is converted from: the GLB's path mirrored from the converted Actors folder into
        /// the staged one, with the NIF extension (the NIF conversion writes every GLB at the mirrored path of its NIF).
        /// </summary>
        public static string RenderSkeletonSource(HavokSkeletonUnit unit, string actorsSource, string actorsOutput) =>
            Path.ChangeExtension(Path.Combine(actorsSource, Path.GetRelativePath(actorsOutput, unit.RenderSkeletonPath)), ".nif");

        /// <summary>Units in label order; nested units such as Character/_1stPerson are separate from their parent.</summary>
        public static List<HavokSkeletonUnit> Discover(string actorsSource, string actorsOutput)
        {
            var units = new List<HavokSkeletonUnit>();
            if (!Directory.Exists(actorsSource)) return units;

            foreach (string skeleton in Directory.EnumerateFiles(actorsSource, HavokCreatureConverter.SkeletonSourceName, SearchOption.AllDirectories))
            {
                string? assets = Path.GetDirectoryName(skeleton);
                if (assets is null || !Path.GetFileName(assets).Equals(HavokCreatureConverter.AssetsFolder, StringComparison.OrdinalIgnoreCase)) continue;

                string unitDir = Path.GetDirectoryName(assets)
                    ?? throw new InvalidOperationException($"Skeleton assets folder '{assets}' has no parent directory.");
                string relative = Path.GetRelativePath(actorsSource, unitDir);
                string outputAssets = Path.Combine(actorsOutput, relative, HavokCreatureConverter.AssetsFolder);
                units.Add(new HavokSkeletonUnit(
                    relative.Replace('\\', '/'), skeleton, Path.Combine(unitDir, HavokCreatureConverter.AnimationsFolder),
                    outputAssets, Path.Combine(outputAssets, HavokCreatureConverter.RenderSkeletonName)));
            }

            foreach (var entry in Overrides)
            {
                string animations = Path.Combine(actorsSource, entry.AnimationsFolder);
                if (!Directory.Exists(animations)) continue;
                units.Add(new HavokSkeletonUnit(
                    entry.Label, Path.Combine(actorsSource, entry.SkeletonHkx), animations,
                    Path.Combine(actorsOutput, entry.OutputFolder), Path.Combine(actorsOutput, entry.RenderSkeleton)));
            }

            return units.OrderBy(u => u.Label, StringComparer.OrdinalIgnoreCase).ToList();
        }
    }
}
