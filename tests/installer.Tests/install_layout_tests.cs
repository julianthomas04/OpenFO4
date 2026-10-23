using AssetConverter;
using Xunit;

namespace InstallerTests
{
    public class InstallLayoutTests
    {
        /// <summary>Pins the installer's spelling of the folder names; a test in the engine's <c>assets/layout.rs</c> compares them with the engine's.</summary>
        [Fact]
        public void TheLayoutNamesAreTheLowercaseOnesTheEngineReads()
        {
            Assert.Equal(
                ["staging", "assets", "meshes", "textures", "interface", "fonts", "layout", "data", "shaders"],
                [
                    InstallLayout.StagingFolder,
                    InstallLayout.AssetsFolder,
                    InstallLayout.MeshesFolder,
                    InstallLayout.TexturesFolder,
                    InstallLayout.InterfaceFolder,
                    InstallLayout.FontsFolder,
                    InstallLayout.LayoutFolder,
                    InstallLayout.DataFolder,
                    InstallLayout.ShadersFolder,
                ]);
        }

        [Fact]
        public void TheAudioFolderIsTheOneTheEngineReads()
        {
            Assert.Equal("audio", InstallLayout.AudioFolder);
        }

        [Fact]
        public void TheVideoFolderIsTheOneTheEngineReads()
        {
            Assert.Equal("video", InstallLayout.VideoFolder);
        }

        [Fact]
        public void TheUserInterfaceFilesLiveBelowTheInterfaceFolderOfTheAssetRoot()
        {
            string assetsRoot = Path.Combine("install", "assets");

            Assert.Equal(Path.Combine("install", "assets", "interface", "fonts"), InstallLayout.FontsDirectory(assetsRoot));
            Assert.Equal(Path.Combine("install", "assets", "interface", "layout"), InstallLayout.UiLayoutDirectory(assetsRoot));
            Assert.Equal("translate.json", InstallLayout.TranslateFileName);
            Assert.Equal("fontconfig.json", InstallLayout.FontConfigFileName);
        }
    }
}
