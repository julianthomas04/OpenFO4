namespace AssetConverter.Video
{
    /// <summary>The game's videos cannot be converted: the video folder or the decoder is missing, a video is malformed or unsupported (the message names it and carries the decoder's stderr), or an output does not match its source.</summary>
    public sealed class VideoConversionException(string message, Exception? inner = null) : ConversionStepException(message, inner);
}
