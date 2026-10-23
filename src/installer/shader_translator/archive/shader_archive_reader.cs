using SharpBSABA2.BA2Util;

namespace AssetConverter.ShaderTranslation.Archive
{
    /// <summary>Reads the shader package out of the game's shader BA2 using the installer's BA2 reader.</summary>
    public static class ShaderArchiveReader
    {
        public const string FxpPath = @"shadersfx\shaders011.fxp";

        public static FxpFile ReadFxp(string ba2Path) => FxpParser.Parse(ReadFxpBytes(ba2Path));

        public static byte[] ReadFxpBytes(string ba2Path)
        {
            if (!File.Exists(ba2Path))
                throw new ShaderArchiveException($"Shader archive not found: {ba2Path}");

            BA2 archive;
            try
            {
                archive = new BA2(ba2Path);
            }
            catch (Exception ex) when (ex is InvalidDataException or IOException or NotSupportedException)
            {
                throw new ShaderArchiveException($"Shader archive {ba2Path} is not a readable BA2: {ex.Message}", ex);
            }

            try
            {
                var matches = archive.Files.Where(f => IsFxpEntry(f.FullPath)).ToList();
                if (matches.Count != 1)
                    throw new ShaderArchiveException(matches.Count == 0
                        ? $"Shader archive {ba2Path} has no entry '{FxpPath}'."
                        : $"Shader archive {ba2Path} has {matches.Count} entries named '{FxpPath}'.");
                return matches[0].GetDataStream().ToArray();
            }
            catch (Exception ex) when (ex is IOException or InvalidDataException or NotSupportedException)
            {
                throw new ShaderArchiveException($"Cannot extract '{FxpPath}' from shader archive {ba2Path}: {ex.Message}", ex);
            }
            finally
            {
                archive.Close();
            }
        }

        private static bool IsFxpEntry(string path)
        {
            string normalized = path.Replace('/', '\\').ToLowerInvariant();
            return normalized == FxpPath || normalized.EndsWith("\\" + FxpPath, StringComparison.Ordinal);
        }
    }
}
