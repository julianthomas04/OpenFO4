namespace AssetConverter
{
    internal static class RecordFlags
    {
        /// <summary>The record is deleted and never spawns in-game.</summary>
        public const int Deleted = 0x20;

        /// <summary>The placed reference starts disabled.</summary>
        public const int InitiallyDisabled = 0x800;
    }

    /// <summary>
    /// Item records never carry the CK-only record flag; their editor-only placeholders are recognised by their model, which lives under
    /// the <c>markers</c> mesh folder.
    /// </summary>
    internal static class MarkerModels
    {
        private const string MarkerFolder = "markers\\";
        private const string MeshesFolder = "meshes\\";

        /// <summary>The value of an <c>is_marker</c> column for an item: 1 when the model path starts in the markers folder; 0 without a model.</summary>
        public static int IsMarker(string? modelPath)
        {
            if (string.IsNullOrEmpty(modelPath)) return 0;
            string path = modelPath.Replace('/', '\\').TrimStart('\\');
            if (path.StartsWith(MeshesFolder, StringComparison.OrdinalIgnoreCase)) path = path[MeshesFolder.Length..];
            return path.StartsWith(MarkerFolder, StringComparison.OrdinalIgnoreCase) ? 1 : 0;
        }
    }
}
