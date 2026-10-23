namespace InstallerTests.Support
{
    /// <summary>Locates checked-in repository files (the engine sources the installer output is contracted with).</summary>
    internal static class RepoFiles
    {
        public static string ReadAllText(string repoRelativePath)
        {
            string root = FindRepoRoot();
            return File.ReadAllText(Path.Combine(root, repoRelativePath));
        }

        /// <summary>First capture group of <paramref name="pattern"/> (dot matches newline); fails with a message naming
        /// the source construct when a contract test's pattern no longer matches the checked-in file.</summary>
        public static string ExtractBlock(string repoRelativePath, string pattern, string description)
        {
            var match = System.Text.RegularExpressions.Regex.Match(
                ReadAllText(repoRelativePath), pattern, System.Text.RegularExpressions.RegexOptions.Singleline);
            if (!match.Success)
                throw new InvalidOperationException($"Contract test cannot find {description} in {repoRelativePath}; the source was restructured, update the test pattern.");
            return match.Groups[1].Value;
        }

        private static string FindRepoRoot()
        {
            var dir = new DirectoryInfo(AppContext.BaseDirectory);
            while (dir != null && !File.Exists(Path.Combine(dir.FullName, "OpenFO4.sln")))
                dir = dir.Parent;
            return dir?.FullName
                ?? throw new DirectoryNotFoundException($"OpenFO4.sln not found above {AppContext.BaseDirectory}");
        }
    }
}
