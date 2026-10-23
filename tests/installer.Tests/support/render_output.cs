namespace InstallerTests
{
    internal static class RenderOutput
    {
        private const string DirectoryVariable = "OPENFO4_TEST_RENDER_DIR";

        private static readonly Lazy<string> ProcessDirectory = new(() => Directory.CreateTempSubdirectory("openfo4-installer-renders-").FullName);

        /// <summary>Where a test writes a rendered image: the directory named by OPENFO4_TEST_RENDER_DIR, else a temp directory private to this test process so parallel runs never share a file.</summary>
        public static string PathFor(string fileName)
        {
            var configured = Environment.GetEnvironmentVariable(DirectoryVariable);
            var directory = string.IsNullOrEmpty(configured) ? ProcessDirectory.Value : Directory.CreateDirectory(configured).FullName;
            return Path.Combine(directory, fileName);
        }
    }
}
