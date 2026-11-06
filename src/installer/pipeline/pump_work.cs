namespace AssetConverter.Pipeline
{
    /// <summary>
    /// Which parts of the sequential game reader an install runs. The reader is one lane but its steps serve different asset groups: the
    /// staged meshes and materials feed four groups, the ESM copy and the texture pixels only the textures and the database. A step that is not
    /// needed still opens its gates (see <see cref="InstallPipeline"/>), so the lanes behind it start as they would have.
    /// </summary>
    /// <param name="IndexTextureNames">Seeds the texture index from the name tables of the texture archives (the meshes and the database read it).</param>
    /// <param name="CopyGameData">Copies the game's ESM, string tables and INIs.</param>
    /// <param name="ExtractTextures">Extracts the texture pixels and the colour LUTs.</param>
    /// <param name="ExtractMeshArchives">Stages the mesh, material and actor animation archives.</param>
    internal readonly record struct PumpWork(bool IndexTextureNames, bool CopyGameData, bool ExtractMeshArchives, bool ExtractTextures)
    {
        private static readonly AssetGroup[] ReadingStagedMeshes = [AssetGroup.Meshes, AssetGroup.Collision, AssetGroup.Animations, AssetGroup.TexturesDatabase];
        private static readonly AssetGroup[] ReadingTextureIndex = [AssetGroup.Meshes, AssetGroup.TexturesDatabase];

        public static PumpWork For(IReadOnlySet<AssetGroup> reconverted) => new(
            IndexTextureNames: ReadingTextureIndex.Any(reconverted.Contains),
            CopyGameData: reconverted.Contains(AssetGroup.TexturesDatabase),
            ExtractMeshArchives: ReadingStagedMeshes.Any(reconverted.Contains),
            ExtractTextures: reconverted.Contains(AssetGroup.TexturesDatabase));
    }
}
