namespace AssetConverter
{
    /// <summary>A font configuration does not map a font class to exactly one family.</summary>
    internal sealed class FontConfigException : Exception
    {
        public FontConfigException(string message) : base(message)
        {
        }
    }
}
