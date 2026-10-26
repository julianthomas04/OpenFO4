using System.Collections.Concurrent;

namespace AssetConverter
{
    /// <summary>The texture paths of a material, backslashed and relative to the textures folder.</summary>
    internal readonly record struct MaterialTextures(string? Diffuse, string? Normal, string? Specular);

    /// <summary>Reads the texture paths out of BGSM and BGEM materials in the extracted materials root; a null result is an unreadable or missing material.</summary>
    internal sealed class MaterialTextureResolver(string? materialsRoot)
    {
        private readonly string? _materialsRoot = materialsRoot;
        private readonly ConcurrentDictionary<string, MaterialTextures?> _matCache = new(); // null value = known miss

        public bool Enabled => _materialsRoot != null;

        public MaterialTextures? Resolve(string mnamPath)
        {
            string rel = mnamPath.Trim().Replace('/', '\\').TrimStart('\\');
            if (rel.StartsWith("materials\\", StringComparison.OrdinalIgnoreCase))
                rel = rel.Substring("materials\\".Length);

            return _matCache.GetOrAdd(rel.ToLowerInvariant(), _ => ParseMaterialFile(rel));
        }

        private MaterialTextures? ParseMaterialFile(string rel)
        {
            if (_materialsRoot == null) return null;
            string path = Path.Combine(_materialsRoot, "Materials", rel);
            if (!File.Exists(path))
            {
                path = Path.Combine(_materialsRoot, rel);
                if (!File.Exists(path)) return null;
            }

            try
            {
                using var stream = new FileStream(path, FileMode.Open, FileAccess.Read);
                using var reader = new BinaryReader(stream);
                uint signature = reader.ReadUInt32();
                stream.Position = 0;

                if (signature == BGSM.Signature)
                {
                    var bgsm = new BGSM();
                    bgsm.Open(stream);
                    return new MaterialTextures(
                        NormalizeTexturePath(bgsm.DiffuseTexture),
                        NormalizeTexturePath(bgsm.NormalTexture),
                        NormalizeTexturePath(bgsm.SmoothSpecTexture));
                }
                if (signature == BGEM.Signature)
                {
                    var bgem = new BGEM();
                    bgem.Open(stream);
                    return new MaterialTextures(
                        NormalizeTexturePath(bgem.BaseTexture),
                        NormalizeTexturePath(bgem.NormalTexture),
                        NormalizeTexturePath(bgem.SpecularTexture));
                }
                return null;
            }
            catch (Exception e) when (BaseMaterialFile.IsReadFailure(e))
            {
                Console.WriteLine($"[WARN] Unreadable material '{path}': {e.Message}");
                return null;
            }
        }

        // Match the TX00-derived row convention: backslashes, relative to the textures folder
        // (the engine prepends "textures/" when loading).
        private static string? NormalizeTexturePath(string? p)
        {
            if (string.IsNullOrWhiteSpace(p)) return null;
            var clean = p.Replace('/', '\\').TrimStart('\\');
            if (clean.StartsWith("textures\\", StringComparison.OrdinalIgnoreCase))
                clean = clean.Substring("textures\\".Length);
            return clean;
        }
    }
}
