namespace AssetConverter
{
    /// <summary>The shared tie-break of the animation exporters: among several sources of one animation the looping one wins, ties go to the first in file order and are counted.</summary>
    internal static class LoopFirstRanking
    {
        public static T? Pick<T>(IReadOnlyList<T> candidates, Func<T, bool> isLooping, string tiesCounter) where T : class
        {
            if (candidates.Count == 0)
                return null;
            var ranked = candidates.Where(isLooping).ToList();
            if (ranked.Count == 0)
                ranked = candidates.ToList();
            if (ranked.Count > 1)
                NifAnimationStats.Add(tiesCounter);
            return ranked[0];
        }
    }
}
