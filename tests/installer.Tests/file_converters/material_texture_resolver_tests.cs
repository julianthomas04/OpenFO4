using AssetConverter;
using InstallerTests.Archives;
using InstallerTests.Support;
using Xunit;

namespace InstallerTests
{
    /// <summary>Reads the console, which it redirects to see the warnings, so it runs serially with the other tests that do.</summary>
    [Collection("Console redirection")]
    public sealed class MaterialTextureResolverTests : IDisposable
    {
        private const string EffectMaterial = @"effects\rain.bgem";

        private readonly TempDirectory _root = new();
        private readonly TextWriter _originalOut = Console.Out;
        private readonly StringWriter _output = new();

        public MaterialTextureResolverTests() => Console.SetOut(_output);

        public void Dispose()
        {
            Console.SetOut(_originalOut);
            _root.Dispose();
        }

        private MaterialTextureResolver Resolver => new(_root.Path);

        private string WriteMaterial(string relativeToRoot, byte[] bytes)
        {
            string path = Path.Combine(_root.Path, relativeToRoot);
            Directory.CreateDirectory(Path.GetDirectoryName(path)!);
            File.WriteAllBytes(path, bytes);
            return path;
        }

        private string WriteEffectMaterial(string relativeToRoot, string baseTexture) =>
            WriteMaterial(relativeToRoot, new BgemFields { BaseTexture = baseTexture }.Build(21));

        [Fact]
        public void Resolve_Bgsm_ReturnsTheDiffuseNormalAndSmoothSpecularTextures()
        {
            WriteMaterial(@"Materials\props\crate.bgsm",
                new BgsmFields { Diffuse = @"props\crate_d.dds", Normal = @"props\crate_n.dds", SmoothSpec = @"props\crate_s.dds" }.Build(2));

            var textures = Resolver.Resolve(@"props\crate.bgsm");

            Assert.Equal(new MaterialTextures(@"props\crate_d.dds", @"props\crate_n.dds", @"props\crate_s.dds"), textures);
        }

        [Fact]
        public void Resolve_Bgem_ReturnsTheBaseNormalAndSpecularTextures()
        {
            WriteMaterial(@"Materials\effects\rain.bgem",
                new BgemFields { BaseTexture = @"effects\rain_d.dds", Normal = @"effects\rain_n.dds", Specular = @"effects\rain_s.dds" }.Build(21));

            var textures = Resolver.Resolve(EffectMaterial);

            Assert.Equal(new MaterialTextures(@"effects\rain_d.dds", @"effects\rain_n.dds", @"effects\rain_s.dds"), textures);
        }

        [Theory]
        [InlineData(@"textures\effects\rain_d.dds")]
        [InlineData(@"TEXTURES\effects\rain_d.dds")]
        [InlineData("textures/effects/rain_d.dds")]
        [InlineData(@"\effects\rain_d.dds")]
        [InlineData(@"effects\rain_d.dds")]
        public void Resolve_TexturePaths_AreBackslashedAndRelativeToTheTexturesFolder(string authored)
        {
            WriteEffectMaterial(@"Materials\effects\rain.bgem", authored);

            Assert.Equal(@"effects\rain_d.dds", Resolver.Resolve(EffectMaterial)?.Diffuse);
        }

        [Fact]
        public void Resolve_EmptyTexturePaths_AreNull()
        {
            WriteEffectMaterial(@"Materials\effects\rain.bgem", @"effects\rain_d.dds");

            var textures = Resolver.Resolve(EffectMaterial);

            Assert.Null(textures?.Normal);
            Assert.Null(textures?.Specular);
        }

        [Theory]
        [InlineData(@"effects\rain.bgem")]
        [InlineData(@"materials\effects\rain.bgem")]
        [InlineData(@"MATERIALS\effects\rain.bgem")]
        [InlineData("Materials/effects/rain.bgem")]
        [InlineData(@"\effects\rain.bgem")]
        [InlineData(@"  effects\rain.bgem  ")]
        public void Resolve_MaterialPathSpellings_ReadTheFileUnderTheMaterialsFolder(string authored)
        {
            WriteEffectMaterial(@"Materials\effects\rain.bgem", @"effects\rain_d.dds");

            Assert.Equal(@"effects\rain_d.dds", Resolver.Resolve(authored)?.Diffuse);
        }

        [Fact]
        public void Resolve_MaterialOutsideTheMaterialsFolder_IsReadFromTheRoot()
        {
            WriteEffectMaterial(@"effects\rain.bgem", @"effects\root_d.dds");

            Assert.Equal(@"effects\root_d.dds", Resolver.Resolve(EffectMaterial)?.Diffuse);
        }

        [Fact]
        public void Resolve_MaterialInBothPlaces_PrefersTheMaterialsFolder()
        {
            WriteEffectMaterial(@"effects\rain.bgem", @"effects\root_d.dds");
            WriteEffectMaterial(@"Materials\effects\rain.bgem", @"effects\materials_d.dds");

            Assert.Equal(@"effects\materials_d.dds", Resolver.Resolve(EffectMaterial)?.Diffuse);
        }

        [Fact]
        public void Resolve_MissingMaterial_IsNullWithoutAWarning()
        {
            Assert.Null(Resolver.Resolve(EffectMaterial));
            Assert.DoesNotContain("[WARN]", _output.ToString());
        }

        [Fact]
        public void Resolve_FileWithoutAMaterialSignature_IsNullWithoutAWarning()
        {
            WriteMaterial(@"Materials\effects\rain.bgem", TestBytes.Random(64, 1).Select(b => (byte)(b | 0x80)).ToArray());

            Assert.Null(Resolver.Resolve(EffectMaterial));
            Assert.DoesNotContain("[WARN]", _output.ToString());
        }

        [Theory]
        [InlineData(0)]
        [InlineData(10)]
        public void Resolve_TruncatedMaterial_WarnsNamingTheFileAndIsNull(int length)
        {
            byte[] whole = new BgemFields { BaseTexture = @"effects\rain_d.dds" }.Build(21);
            string path = WriteMaterial(@"Materials\effects\rain.bgem", whole[..length]);

            Assert.Null(Resolver.Resolve(EffectMaterial));
            Assert.Contains($"[WARN] Unreadable material '{path}'", _output.ToString());
        }

        [Fact]
        public void Resolve_SamePathAgain_ReturnsTheFirstResultWithoutReadingTheFileAgain()
        {
            string path = WriteEffectMaterial(@"Materials\effects\rain.bgem", @"effects\first_d.dds");
            var resolver = Resolver;
            Assert.Equal(@"effects\first_d.dds", resolver.Resolve(EffectMaterial)?.Diffuse);

            File.Delete(path);

            Assert.Equal(@"effects\first_d.dds", resolver.Resolve(EffectMaterial)?.Diffuse);
        }

        [Fact]
        public void Resolve_PathsDifferingInSpellingAndCase_ShareOneCacheEntry()
        {
            string path = WriteEffectMaterial(@"Materials\effects\rain.bgem", @"effects\first_d.dds");
            var resolver = Resolver;
            Assert.Equal(@"effects\first_d.dds", resolver.Resolve(EffectMaterial)?.Diffuse);

            File.Delete(path);

            Assert.Equal(@"effects\first_d.dds", resolver.Resolve(@"Materials/Effects/RAIN.BGEM")?.Diffuse);
        }

        [Fact]
        public void Resolve_MissingMaterialThatAppearsLater_StaysAMiss()
        {
            var resolver = Resolver;
            Assert.Null(resolver.Resolve(EffectMaterial));

            WriteEffectMaterial(@"Materials\effects\rain.bgem", @"effects\rain_d.dds");

            Assert.Null(resolver.Resolve(EffectMaterial));
        }

        [Fact]
        public void WithoutAMaterialsRoot_ResolverIsDisabledAndResolvesNothing()
        {
            WriteEffectMaterial(@"Materials\effects\rain.bgem", @"effects\rain_d.dds");
            var disabled = new MaterialTextureResolver(null);

            Assert.False(disabled.Enabled);
            Assert.Null(disabled.Resolve(EffectMaterial));
            Assert.True(Resolver.Enabled);
        }
    }
}
