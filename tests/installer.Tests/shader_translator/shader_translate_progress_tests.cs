using AssetConverter.Progress;
using AssetConverter.ShaderTranslation;
using AssetConverter.ShaderTranslation.Archive;
using AssetConverter.ShaderTranslation.Dxbc;
using AssetConverter.ShaderTranslation.Installation;
using Xunit;

namespace InstallerTests.ShaderTranslation
{
    // Drives the process-wide InstallProgress, so it runs serially with the other tests that use it.
    [Collection("Console redirection")]
    public sealed class ShaderTranslateProgressTests : IDisposable
    {
        public ShaderTranslateProgressTests() => InstallProgress.Start([new StepDefinition(InstallSteps.ConvertShaders)]);

        public void Dispose() => InstallProgress.Stop();

        private static FxpFile Archive() =>
            new([], [new FxpEntry(0, 0, 0, ReadOnlyMemory<byte>.Empty, FxpBuilder.TinyDxbc(ShaderStage.Vertex))]);

        private static ShaderManifest Manifest(int shaders) => ShaderManifestReader.Parse(
            "{\"shaders\":[" + string.Join(",", Enumerable.Range(0, shaders).Select(i => $"{{\"name\":\"tiny_{i}_vs\",\"blob\":0,\"stage\":\"vertex\"}}")) + "]}");

        private static StepSnapshot Step() => InstallProgress.Model!.GetSnapshot().Steps.Single();

        [Fact]
        public void Translate_TicksOncePerTranslatedShaderAndEndsAtTheShaderCount()
        {
            int phase = InstallProgress.BeginPhase(InstallSteps.ConvertShaders, 0);
            using var root = new TempAssetRoot();

            int count = ShaderTranslateStep.Translate(Archive(), Manifest(5), root.Path, phase);

            Assert.Equal(5, count);
            Assert.Equal(5, Step().Total);
            Assert.Equal(5, Step().Completed);
        }

        [Fact]
        public void Translate_Cancelled_ThrowsCancellationAndLeavesNoShadersFolder()
        {
            using var root = new TempAssetRoot();
            using var cts = new CancellationTokenSource();
            cts.Cancel();

            Assert.ThrowsAny<OperationCanceledException>(
                () => ShaderTranslateStep.Translate(Archive(), Manifest(3), root.Path, new DirectoryOperations(), _ => { }, -1, cts.Token));

            Assert.False(Directory.Exists(root.Shaders));
        }
    }
}
