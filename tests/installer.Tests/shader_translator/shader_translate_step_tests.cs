using System.Security.Cryptography;
using System.Text.Json;
using System.Text.RegularExpressions;
using AssetConverter;
using AssetConverter.ShaderTranslation;
using AssetConverter.ShaderTranslation.Archive;
using AssetConverter.ShaderTranslation.Dxbc;
using AssetConverter.ShaderTranslation.Installation;
using InstallerTests.Support;
using Xunit;
using static InstallerTests.Support.Sm5Assembler;

namespace InstallerTests.ShaderTranslation
{
    public sealed class TempAssetRoot : IDisposable
    {
        public string Path { get; } = System.IO.Path.Combine(System.IO.Path.GetTempPath(), "shader-step-" + Guid.NewGuid().ToString("N"));

        public TempAssetRoot() => Directory.CreateDirectory(Path);

        public string Shaders => System.IO.Path.Combine(Path, InstallLayout.ShadersFolder);

        public void Dispose() => Directory.Delete(Path, recursive: true);
    }

    public class ShaderTranslateStepTests
    {
        private static readonly uint[] AllComponents = Swz(X, Y, Z, W);

        private static byte[] PixelCopyingCb0() =>
            new Sm5Assembler(ShaderStage.Pixel).ConstantBufferDeclaration(0, 1).OutputDeclaration(0, 0xF).Temps(1)
                .Instruction(Opcode.Mov, 0, Dst(OperandType.Output, 0, 0xF), ConstantBuffer(0, 0, AllComponents))
                .Instruction(Opcode.Ret, 0).BuildDxbc();

        private static byte[] TinyVertex() => FxpBuilder.TinyDxbc(ShaderStage.Vertex);

        private static byte[] UnsupportedPixel() =>
            new Sm5Assembler(ShaderStage.Pixel).Temps(1)
                .Instruction(Opcode.Lod, 0, Dst(OperandType.Temp, 0, 0b0001), Src(OperandType.Temp, 0, AllComponents), Resource(0, AllComponents), Sampler(0))
                .Instruction(Opcode.Ret, 0).BuildDxbc();

        private static FxpFile Archive(params byte[][] blobs) =>
            new([], blobs.Select((b, i) => new FxpEntry(i, 0, 0, ReadOnlyMemory<byte>.Empty, b)).ToList());

        private static ShaderManifest Manifest(string json) => ShaderManifestReader.Parse(json);

        private const string PixelEntry = """{"name":"copy_ps","blob":1,"stage":"fragment","bindings":[{"register":"cb0","name":"copy_cb0","group":1,"binding":2}]}""";
        private const string VertexEntry = """{"name":"tiny_vs","blob":0,"stage":"vertex"}""";

        private static ShaderManifest TwoShaderManifest() => Manifest($$"""{"shaders":[{{PixelEntry}},{{VertexEntry}}]}""");

        [Fact]
        public void Translate_WritesWgslFilesAndManifestWithProvenance()
        {
            using var root = new TempAssetRoot();
            byte[] pixel = PixelCopyingCb0();

            int count = ShaderTranslateStep.Translate(Archive(TinyVertex(), pixel), TwoShaderManifest(), root.Path);

            Assert.Equal(2, count);
            string wgsl = File.ReadAllText(Path.Combine(root.Shaders, "copy_ps.wgsl"));
            Assert.Contains("@group(1) @binding(2) var<uniform> copy_cb0: array<vec4<f32>, 1>;", wgsl);
            Assert.Contains("fn copy_ps(i: copy_ps_in) -> copy_ps_out {", wgsl);
            Assert.True(File.Exists(Path.Combine(root.Shaders, "tiny_vs.wgsl")));

            using var doc = JsonDocument.Parse(File.ReadAllText(Path.Combine(root.Shaders, "manifest.json")));
            Assert.Equal(ShaderTranslatorVersion.Value, doc.RootElement.GetProperty("translator_version").GetUInt32());
            var shaders = doc.RootElement.GetProperty("shaders").EnumerateArray().ToList();
            Assert.Equal(["copy_ps", "tiny_vs"], shaders.Select(s => s.GetProperty("name").GetString()));
            var copy = shaders[0];
            Assert.Equal("copy_ps.wgsl", copy.GetProperty("file").GetString());
            Assert.Equal("fragment", copy.GetProperty("stage").GetString());
            Assert.Equal(1, copy.GetProperty("source_blob").GetInt32());
            Assert.Equal(Convert.ToHexString(SHA256.HashData(pixel)).ToLowerInvariant(), copy.GetProperty("source_sha256").GetString());
            Assert.Equal("vertex", shaders[1].GetProperty("stage").GetString());
        }

        [Fact]
        public void Translate_EmptyManifest_WritesEmptyManifest()
        {
            using var root = new TempAssetRoot();

            ShaderTranslateStep.Translate(Archive(), Manifest("""{"shaders":[]}"""), root.Path);

            using var doc = JsonDocument.Parse(File.ReadAllText(Path.Combine(root.Shaders, "manifest.json")));
            Assert.Empty(doc.RootElement.GetProperty("shaders").EnumerateArray());
        }

        [Fact]
        public void Translate_PublishesIntoTheShadersFolderOfTheAssetRoot_AndCreatesNoCacheFolder()
        {
            using var root = new TempAssetRoot();

            ShaderTranslateStep.Translate(Archive(TinyVertex()), Manifest($"{{\"shaders\":[{VertexEntry}]}}"), root.Path);

            Assert.True(File.Exists(Path.Combine(root.Shaders, "manifest.json")));
            Assert.True(File.Exists(Path.Combine(root.Shaders, "tiny_vs.wgsl")));
            Assert.False(Directory.Exists(Path.Combine(root.Path, "cache")));
        }

        [Fact]
        public void Translate_SecondRun_ReplacesShadersFolderAndRemovesStaleFiles()
        {
            using var root = new TempAssetRoot();
            ShaderTranslateStep.Translate(Archive(TinyVertex(), PixelCopyingCb0()), TwoShaderManifest(), root.Path);

            ShaderTranslateStep.Translate(Archive(TinyVertex()), Manifest($"{{\"shaders\":[{VertexEntry}]}}"), root.Path);

            Assert.Equal(["manifest.json", "tiny_vs.wgsl"], Directory.GetFiles(root.Shaders).Select(Path.GetFileName).Order());
            Assert.Equal([InstallLayout.ShadersFolder], Directory.GetDirectories(root.Path).Select(Path.GetFileName));
        }

        [Fact]
        public void Translate_BlobMissingFromArchive_ThrowsAndLeavesNoShadersFolder()
        {
            using var root = new TempAssetRoot();

            var ex = Assert.Throws<ShaderTranslateStepException>(() =>
                ShaderTranslateStep.Translate(Archive(TinyVertex()), TwoShaderManifest(), root.Path));

            Assert.Contains("copy_ps", ex.Message);
            Assert.Contains("blob 1", ex.Message);
            Assert.False(Directory.Exists(root.Shaders));
            Assert.Empty(Directory.GetDirectories(root.Path));
        }

        [Fact]
        public void Translate_FailureAfterEarlierShadersSucceeded_KeepsPreviousShadersFolderIntact()
        {
            using var root = new TempAssetRoot();
            ShaderTranslateStep.Translate(Archive(TinyVertex(), PixelCopyingCb0()), TwoShaderManifest(), root.Path);
            string before = File.ReadAllText(Path.Combine(root.Shaders, "manifest.json"));
            var failing = Manifest($$"""{"shaders":[{{VertexEntry}},{"name":"bad_ps","blob":2,"stage":"fragment"}]}""");

            var ex = Assert.Throws<ShaderTranslateStepException>(() =>
                ShaderTranslateStep.Translate(Archive(TinyVertex(), PixelCopyingCb0(), UnsupportedPixel()), failing, root.Path));

            Assert.IsType<WgslUnsupportedException>(ex.InnerException);
            Assert.Contains("bad_ps", ex.Message);
            Assert.Equal(before, File.ReadAllText(Path.Combine(root.Shaders, "manifest.json")));
            Assert.True(File.Exists(Path.Combine(root.Shaders, "copy_ps.wgsl")));
            Assert.Equal([InstallLayout.ShadersFolder], Directory.GetDirectories(root.Path).Select(Path.GetFileName));
        }

        [Fact]
        public void Translate_StageMismatchBetweenManifestAndBlob_Throws()
        {
            using var root = new TempAssetRoot();
            var manifest = Manifest("""{"shaders":[{"name":"x","blob":0,"stage":"fragment"}]}""");

            var ex = Assert.Throws<ShaderTranslateStepException>(() => ShaderTranslateStep.Translate(Archive(TinyVertex()), manifest, root.Path));

            Assert.Contains("Vertex", ex.Message);
        }

        [Fact]
        public void Translate_RegisterMissingFromManifestBindings_ThrowsUnmappedResourceWithShaderContext()
        {
            using var root = new TempAssetRoot();
            var manifest = Manifest("""{"shaders":[{"name":"copy_ps","blob":0,"stage":"fragment"}]}""");

            var ex = Assert.Throws<ShaderTranslateStepException>(() => ShaderTranslateStep.Translate(Archive(PixelCopyingCb0()), manifest, root.Path));

            Assert.IsType<UnmappedResourceException>(ex.InnerException);
            Assert.False(Directory.Exists(root.Shaders));
        }

        [Fact]
        public void Translate_CorruptBlob_ThrowsTypedError()
        {
            using var root = new TempAssetRoot();

            Assert.Throws<ShaderTranslateStepException>(() =>
                ShaderTranslateStep.Translate(Archive([1, 2, 3]), Manifest("""{"shaders":[{"name":"x","blob":0,"stage":"vertex"}]}"""), root.Path));
        }

        [Fact]
        public void Translate_LeftoverTempDirectoriesOfKilledRuns_AreRemoved()
        {
            using var root = new TempAssetRoot();
            string stale = Path.Combine(root.Path, InstallLayout.ShadersFolder + ".tmp-deadbeef");
            string staleBackup = Path.Combine(root.Path, InstallLayout.ShadersFolder + ".old-deadbeef");
            Directory.CreateDirectory(stale);
            Directory.CreateDirectory(staleBackup);

            ShaderTranslateStep.Translate(Archive(TinyVertex()), Manifest($"{{\"shaders\":[{VertexEntry}]}}"), root.Path);

            Assert.False(Directory.Exists(stale));
            Assert.False(Directory.Exists(staleBackup));
        }

        [Fact]
        public void Translate_ExternalDottedBinding_IsPastedVerbatim()
        {
            using var root = new TempAssetRoot();
            var manifest = Manifest("""{"shaders":[{"name":"copy_ps","blob":0,"stage":"fragment","bindings":[{"register":"cb0","name":"frame.cb0","group":0,"binding":0,"external":true}]}]}""");

            ShaderTranslateStep.Translate(Archive(PixelCopyingCb0()), manifest, root.Path);

            string wgsl = File.ReadAllText(Path.Combine(root.Shaders, "copy_ps.wgsl"));
            Assert.Contains("o.o0 = frame.cb0[0].xyzw;", wgsl);
            Assert.DoesNotContain("var<uniform>", wgsl);
        }

        [Fact]
        public void Run_MissingShaderArchive_ThrowsArchiveException()
        {
            using var root = new TempAssetRoot();
            string manifestPath = Path.Combine(root.Path, "m.json");
            File.WriteAllText(manifestPath, """{"shaders":[]}""");

            Assert.Throws<ShaderArchiveException>(() => ShaderTranslateStep.Run(Path.Combine(root.Path, "no-game"), root.Path, manifestPath, -1, TestContext.Current.CancellationToken));
        }

        [Fact]
        public void TranslatorVersion_MatchesTheEngineConstant()
        {
            string rust = RepoFiles.ReadAllText("src/engine/src/rendering/shaders/translated.rs");
            var match = Regex.Match(rust, @"pub\s+const\s+TRANSLATOR_VERSION\s*:\s*u32\s*=\s*(\d+)\s*;");

            Assert.True(match.Success, "TRANSLATOR_VERSION not found in translated.rs; update this contract test if the constant moved.");
            Assert.Equal(ShaderTranslatorVersion.Value, uint.Parse(match.Groups[1].Value, System.Globalization.CultureInfo.InvariantCulture));
        }
    }
}
