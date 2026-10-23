namespace AssetConverter
{
    /// <summary>
    /// The files that make up the game itself (the executable, the runtime libraries it ships with and the folders of plugins beside them) and whether the install cannot do without them.
    /// With a <paramref name="Root"/>, a file is installed at its path below it; without one, every file lands in the install folder under its own name.
    /// </summary>
    internal sealed record GameFiles(IReadOnlyList<string> Files, bool Required, string? Root = null)
    {
        public const string ExecutableName = "openfo4.exe";

        public static GameFiles None { get; } = new([], Required: false);

        public bool HasExecutable => Files.Any(file => InstalledPath(file).Equals(ExecutableName, StringComparison.OrdinalIgnoreCase));

        /// <summary>The path of <paramref name="file"/> below the install folder.</summary>
        public string InstalledPath(string file) => Root == null ? Path.GetFileName(file) : Path.GetRelativePath(Root, file);
    }
}
