namespace AssetConverter
{
    /// <summary>The cap on threads for work that is bound by file-system latency rather than CPU.</summary>
    internal static class IoParallelism
    {
        public const int MaxThreads = 16;
    }
}
