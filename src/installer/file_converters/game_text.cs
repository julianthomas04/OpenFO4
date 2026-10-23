namespace AssetConverter
{
    /// <summary>The text files of the game's archives (font configurations, translate tables, INI files): UTF-8 unless a byte order mark says otherwise.</summary>
    internal static class GameText
    {
        public static string Decode(byte[] content)
        {
            using var reader = new StreamReader(new MemoryStream(content), System.Text.Encoding.UTF8, detectEncodingFromByteOrderMarks: true);
            return reader.ReadToEnd();
        }
    }
}
