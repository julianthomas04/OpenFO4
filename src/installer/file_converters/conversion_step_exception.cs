namespace AssetConverter
{
    /// <summary>
    /// A failure of an interface conversion step that the step names and explains: the dev commands report it like a failed install step
    /// (the message and a failure exit code) instead of dumping a stack trace.
    /// </summary>
    public abstract class ConversionStepException(string message, Exception? inner = null) : Exception(message, inner);

    /// <summary>The vendored ffdec cannot be found, started, or ran and failed.</summary>
    public sealed class FfdecException(string message, Exception? inner = null) : ConversionStepException(message, inner);

    /// <summary>The Librempeg ffmpeg cannot be found, started, or ran and failed.</summary>
    public sealed class FfmpegException(string message, Exception? inner = null) : ConversionStepException(message, inner);
}
