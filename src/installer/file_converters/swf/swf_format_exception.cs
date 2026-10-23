namespace AssetConverter
{
    /// <summary>A SWF movie is malformed, or holds a structure its reader cannot interpret without guessing.</summary>
    public sealed class SwfFormatException(string message) : Exception(message) { }
}
