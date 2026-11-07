using AssetConverter.InstallerUi;
using Xunit;

namespace InstallerTests.Ui
{
    public sealed class GameLauncherTests : IDisposable
    {
        private const string InstallDir = "OPENFO4_INSTALL_DIR";
        private const string AssetRoot = "OPENFO4_ASSET_ROOT";
        private const string Unrelated = "OPENFO4_GAME_LAUNCHER_TEST_UNRELATED";

        public GameLauncherTests()
        {
            Environment.SetEnvironmentVariable(InstallDir, @"D:\install");
            Environment.SetEnvironmentVariable(AssetRoot, @"D:\install\assets");
            Environment.SetEnvironmentVariable(Unrelated, "kept");
        }

        public void Dispose()
        {
            Environment.SetEnvironmentVariable(InstallDir, null);
            Environment.SetEnvironmentVariable(AssetRoot, null);
            Environment.SetEnvironmentVariable(Unrelated, null);
        }

        [Fact]
        public void TheGameDoesNotInheritTheVariablesThatPointTheEngineAtAnotherInstall()
        {
            var startInfo = ProcessGameLauncher.StartInfo(@"C:\OpenFO4\openfo4.exe", @"C:\OpenFO4");

            Assert.False(startInfo.Environment.ContainsKey(InstallDir));
            Assert.False(startInfo.Environment.ContainsKey(AssetRoot));
            Assert.Equal("kept", startInfo.Environment[Unrelated]);
        }

        [Fact]
        public void TheGameStartsInTheInstallFolderWithoutAConsoleWindowOrTheShell()
        {
            var startInfo = ProcessGameLauncher.StartInfo(@"C:\OpenFO4\openfo4.exe", @"C:\OpenFO4");

            Assert.Equal(@"C:\OpenFO4\openfo4.exe", startInfo.FileName);
            Assert.Equal(@"C:\OpenFO4", startInfo.WorkingDirectory);
            Assert.True(startInfo.CreateNoWindow);
            Assert.False(startInfo.UseShellExecute);
        }
    }
}
