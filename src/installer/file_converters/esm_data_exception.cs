namespace AssetConverter
{
    /// <summary>
    /// A record holds data the importer cannot represent in the database (an unknown record variant, a condition shape without a column):
    /// the import stops and names the record instead of storing a guess.
    /// </summary>
    public sealed class UnsupportedEsmDataException(string message, Exception? inner = null) : ConversionStepException(message, inner);
}
