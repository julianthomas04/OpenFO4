namespace AssetConverter
{
    /// <summary>How archive entry paths are compared and classified: either slash separates, case does not matter.</summary>
    internal static class ArchivePath
    {
        public static IEqualityComparer<string> Comparer { get; } = new NormalizedComparer();

        public static string Normalize(string path) => path.Replace('/', '\\');

        /// <summary>Whether <paramref name="path"/> lies below <paramref name="prefix"/> (backslash separated, ending in a separator) and has <paramref name="extension"/>.</summary>
        public static bool Matches(string path, string prefix, string extension)
        {
            string normalized = Normalize(path);
            return normalized.StartsWith(prefix, StringComparison.OrdinalIgnoreCase)
                && normalized.EndsWith(extension, StringComparison.OrdinalIgnoreCase);
        }

        private sealed class NormalizedComparer : IEqualityComparer<string>
        {
            public bool Equals(string? x, string? y) =>
                string.Equals(x == null ? null : Normalize(x), y == null ? null : Normalize(y), StringComparison.OrdinalIgnoreCase);

            public int GetHashCode(string path) => string.GetHashCode(Normalize(path), StringComparison.OrdinalIgnoreCase);
        }
    }
}
