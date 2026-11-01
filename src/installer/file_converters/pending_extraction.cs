namespace AssetConverter
{
    /// <summary>
    /// Work an extractor found out of date while planning, which it has not started: what it is called in the progress detail and the
    /// action that does it. An extractor whose output is current has no pending extraction. The action reads its input again, so a plan
    /// holds only what it needed to decide.
    /// </summary>
    internal sealed record PendingExtraction(string Detail, Action Run)
    {
        /// <summary>Runs the extraction if there is one; returns whether there was.</summary>
        public static bool RunIfPending(PendingExtraction? pending)
        {
            pending?.Run();
            return pending != null;
        }
    }
}
