using AssetConverter.ShaderTranslation.Archive;
using AssetConverter.ShaderTranslation.Dxbc;

namespace AssetConverter.ShaderTranslation.Installation
{
    /// <summary>Expands the manifest's families into one <see cref="ManifestShader"/> per blob of the family's FXP group.</summary>
    public static class ShaderFamilyExpander
    {
        public static IReadOnlyList<ManifestShader> Expand(FxpFile fxp, ShaderManifest manifest)
        {
            var shaders = new List<ManifestShader>(manifest.Shaders);
            foreach (var family in manifest.Families)
                shaders.AddRange(ExpandFamily(fxp, family));
            if (manifest.Families.Count > 0)
                ShaderManifestReader.CheckShaderNames(shaders, "shader manifest (after family expansion)");
            return shaders;
        }

        private static List<ManifestShader> ExpandFamily(FxpFile fxp, ManifestFamily family)
        {
            string label = $"family '{family.Family}' (FXP group {family.FxpGroup})";
            if (family.FxpGroup >= fxp.Groups.Count)
                throw new ShaderTranslateStepException($"{label}: the shader archive has only {fxp.Groups.Count} groups.");
            var header = fxp.Groups[family.FxpGroup];
            if (header.VertexShaderCount != family.VertexCount || header.PixelShaderCount != family.PixelCount)
                throw new ShaderTranslateStepException(
                    $"{label}: the group header says {header.VertexShaderCount} vertex / {header.PixelShaderCount} pixel shaders but the manifest expects {family.VertexCount} / {family.PixelCount}; this is a different game build.");

            var entries = fxp.Entries.Where(e => e.GroupIndex == family.FxpGroup).OrderBy(e => e.BlobId).ToList();
            if (entries.Count != family.VertexCount + family.PixelCount)
                throw new ShaderTranslateStepException(
                    $"{label}: the group holds {entries.Count} entries but the manifest expects {family.VertexCount + family.PixelCount}.");

            var seen = new Dictionary<(ShaderStage, uint), int>();
            var shaders = new List<ManifestShader>(entries.Count);
            for (int i = 0; i < entries.Count; i++)
            {
                var entry = entries[i];
                bool vertex = i < family.VertexCount;
                var stage = vertex ? ShaderStage.Vertex : ShaderStage.Pixel;
                if (!seen.TryAdd((stage, entry.Descriptor), entry.BlobId))
                    throw new ShaderTranslateStepException(
                        $"{label}: descriptor 0x{entry.Descriptor:x8} appears twice among {(vertex ? "vertex" : "pixel")} shaders (blobs {seen[(stage, entry.Descriptor)]} and {entry.BlobId}).");
                shaders.Add(new ManifestShader(
                    ManifestFamily.ExpandName(family.NameTemplate, vertex ? "vs" : "ps", entry.Descriptor),
                    entry.BlobId,
                    stage,
                    family.NanPolicy,
                    vertex ? family.VertexBindings : family.PixelBindings));
            }
            return shaders;
        }
    }
}
