namespace AssetConverter.Audio
{
    /// <summary>The game's audio cannot be converted: an archive is missing or unreadable, an entry cannot be converted (the message names it), or two entries would write the same file.</summary>
    public sealed class AudioConversionException(string message, Exception? inner = null) : ConversionStepException(message, inner);
}
