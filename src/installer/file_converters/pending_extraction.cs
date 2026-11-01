namespace AssetConverter
{
    /// <summary>
    /// Work an extractor found out of date while planning, which it has not started: what it is called in the progress detail and the
    /// action that does it. An extractor whose output is current has no pending extraction.
    /// </summary>
    internal sealed record PendingExtraction(string Detail, Action Run);
}
