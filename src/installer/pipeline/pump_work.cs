namespace AssetConverter.Pipeline
{
    /// <summary>
    /// Which parts of the sequential game reader an install runs. The reader is one lane but its steps serve different asset groups: the
    /// staged meshes and materials feed four groups, the ESM copy and the texture pixels only the textures and the database. A step that is not
    /// needed still opens its gates (see <see cref="InstallPipeline"/>), so the lanes behind it start as they would have. The texture
    /// index is part of the texture work: the meshes and the database read it, and a meshes change always converts the textures and the database too.
    /// </summary>
    /// <param name="CopyGameData">Copies the game's ESM and extracts the string tables.</param>
    /// <param name="ExtractMeshArchives">Stages the mesh, material and actor animation archives.</param>
    /// <param name="ExtractTextures">Indexes the texture names, then extracts the texture pixels and the colour LUTs.</param>
    internal readonly record struct PumpWork(bool CopyGameData, bool ExtractMeshArchives, bool ExtractTextures)
    {
        private static readonly AssetGroup[] ReadingStagedMeshes = [AssetGroup.Meshes, AssetGroup.Collision, AssetGroup.Animations, AssetGroup.TexturesDatabase];

        /// <summary>The groups, among those converted, that read what the mesh archive extraction stages; a failure of that extraction leaves them all unfinished.</summary>
        public static IReadOnlyList<AssetGroup> MeshExtractionConsumers(IReadOnlySet<AssetGroup> reconverted) =>
            [.. ReadingStagedMeshes.Where(reconverted.Contains)];

        public static PumpWork For(IReadOnlySet<AssetGroup> reconverted) => new(
            CopyGameData: reconverted.Contains(AssetGroup.TexturesDatabase),
            ExtractMeshArchives: ReadingStagedMeshes.Any(reconverted.Contains),
            ExtractTextures: reconverted.Contains(AssetGroup.TexturesDatabase));
    }
}
