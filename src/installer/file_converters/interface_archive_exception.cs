namespace AssetConverter
{
    /// <summary>The game's interface archive cannot be opened, lacks an entry, or did not yield an entry it was asked for.</summary>
    public sealed class InterfaceArchiveException : ConversionStepException
    {
        public InterfaceArchiveException(string message, Exception? inner = null) : base(message, inner)
        {
        }
    }
}
