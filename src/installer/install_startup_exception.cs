namespace AssetConverter
{
    /// <summary>The installer could not start. The message says what to do about it and is meant to be shown to the user as it is.</summary>
    public sealed class InstallStartupException(string message, Exception? inner = null) : Exception(message, inner);
}
