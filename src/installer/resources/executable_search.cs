namespace AssetConverter
{
    internal static class ExecutableSearch
    {
        /// <returns>The full path of the first <paramref name="executable"/> in a directory of <paramref name="pathVariable"/>, or null when there is none.</returns>
        public static string? FindOnPath(string executable, string? pathVariable) =>
            (pathVariable ?? "").Split(Path.PathSeparator, StringSplitOptions.RemoveEmptyEntries | StringSplitOptions.TrimEntries)
                .Select(directory => directory.Trim('"'))
                .Where(directory => directory.Length > 0)
                .Select(directory => Path.Combine(directory, executable))
                .FirstOrDefault(File.Exists);
    }
}
