namespace AssetConverter.Pipeline
{
    /// <summary>The files of one folder that match <paramref name="Pattern"/>, as <see cref="InstallOperations.PurgeByExtension"/> deletes them.</summary>
    internal readonly record struct PurgeStep(string Directory, string Pattern, string Description, SearchOption Scope = SearchOption.AllDirectories);

    /// <summary>
    /// What an asset group deletes before it is converted again. Purging makes a conversion start from nothing, which also defeats the conversion stamps that sit in
    /// the output folders (a stamp that survived would let the converter keep output the revision bump meant to replace). Steps are patterns, not whole
    /// folders, because groups share folders: collision sidecars live among the GLBs.
    /// Three groups purge elsewhere: the meshes delete their GLBs in the conversion itself, right before the render skeletons that other lanes wait for;
    /// each animation unit deletes its own clip packs and index before it converts; the shader step replaces its folder as a whole when it publishes.
    /// </summary>
    internal static class GroupPurge
    {
        private const string EveryFile = "*";

        public static IReadOnlyList<PurgeStep> StepsFor(AssetGroup group, InstallLocations paths) => group switch
        {
            AssetGroup.Interface =>
            [
                new(paths.FontsOut, EveryFile, "interface fonts"),
                new(paths.UiLayoutOut, EveryFile, "UI layout documents"),
                new(paths.PipboyArtOut, EveryFile, "Pip-Boy art"),
            ],
            AssetGroup.Audio => [new(paths.AudioOut, EveryFile, "converted audio files")],
            AssetGroup.Video => [new(paths.VideoOut, EveryFile, "converted video files")],
            AssetGroup.Collision =>
            [
                new(paths.MeshesOut, CollisionSidecars.StampFileName, "collision stamp", SearchOption.TopDirectoryOnly),
                new(paths.MeshesOut, CollisionSidecars.SidecarPattern, "collision sidecars"),
            ],
            AssetGroup.TexturesDatabase =>
            [
                new(paths.TexturesOut, EveryFile, "textures"),
                .. DatabaseFiles(paths),
                new(paths.StagingRoot, "*" + TextureArchiveSet.MarkerSuffix, "texture extraction markers", SearchOption.TopDirectoryOnly),
            ],
            _ => [],
        };

        public static void Purge(AssetGroup group, InstallLocations paths, Action<TimeSpan>? wait = null, CancellationToken ct = default)
        {
            foreach (var step in StepsFor(group, paths))
                InstallOperations.PurgeByExtension(step.Directory, step.Pattern, step.Description, step.Scope, wait, ct);
        }

        private static IEnumerable<PurgeStep> DatabaseFiles(InstallLocations paths)
        {
            string name = Path.GetFileName(paths.DatabasePath);
            return new[] { name, name + "-shm", name + "-wal" }
                .Select(file => new PurgeStep(paths.DataOut, file, "database files", SearchOption.TopDirectoryOnly));
        }
    }
}
