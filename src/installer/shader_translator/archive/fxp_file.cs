namespace AssetConverter.ShaderTranslation.Archive
{
    public sealed record FxpGroupHeader(uint VertexShaderCount, uint Field1, uint Field2, uint PixelShaderCount, uint Field4);

    /// <summary>One shader entry of an FXP. <see cref="BlobId"/> is the global 0-based index across all groups.</summary>
    public sealed record FxpEntry(
        int BlobId,
        int GroupIndex,
        uint Descriptor,
        ReadOnlyMemory<byte> SlotTable,
        ReadOnlyMemory<byte> Dxbc);

    public sealed class FxpFile
    {
        public IReadOnlyList<FxpGroupHeader> Groups { get; }
        public IReadOnlyList<FxpEntry> Entries { get; }

        public FxpFile(IReadOnlyList<FxpGroupHeader> groups, IReadOnlyList<FxpEntry> entries)
        {
            Groups = groups;
            Entries = entries;
        }
    }
}
