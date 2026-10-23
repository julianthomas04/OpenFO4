using AssetConverter.ShaderTranslation;
using AssetConverter.ShaderTranslation.Dxbc;
using AssetConverter.ShaderTranslation.Installation;
using AssetConverter.ShaderTranslation.Wgsl;
using InstallerTests.Support;
using Xunit;

namespace InstallerTests.ShaderTranslation
{
    public class ShaderManifestReaderTests
    {
        private static ShaderManifest Parse(string json) => ShaderManifestReader.Parse(json);

        private static string OneShader(string shaderFields = "", string bindings = "") =>
            $$"""{"shaders":[{"name":"water_ps","blob":12,"stage":"fragment"{{shaderFields}}{{(bindings.Length > 0 ? $",\"bindings\":[{bindings}]" : "")}}}]}""";

        private static string Binding(string register = "cb0", string name = "water_cb0", string extra = "") =>
            $$"""{"register":"{{register}}","name":"{{name}}","group":1,"binding":3{{extra}}}""";

        [Fact]
        public void Parse_EmptyShaderList_IsValid()
        {
            Assert.Empty(Parse("""{"shaders":[]}""").Shaders);
        }

        [Fact]
        public void Parse_FullEntry_MapsEveryField()
        {
            var manifest = Parse("""
                {"shaders":[{"name":"hiz","blob":3860,"stage":"compute","nan_policy":"d3d_strict","bindings":[
                  {"register":"cb0","name":"hiz_cb0","group":0,"binding":0},
                  {"register":"t0","name":"hiz_depth","group":0,"binding":1,"texture_override":"depth"},
                  {"register":"s2","name":"hiz_s2","group":0,"binding":2},
                  {"register":"u1","name":"hiz_out","group":0,"binding":3,"uav_format":"r32float"},
                  {"register":"cb12","name":"frame.cb12","group":2,"binding":0,"external":true}]}]}
                """);

            var shader = Assert.Single(manifest.Shaders);
            Assert.Equal(("hiz", 3860, ShaderStage.Compute, NanPolicy.D3dStrict), (shader.Name, shader.Blob, shader.Stage, shader.NanPolicy));
            Assert.Equal(
                [
                    new ManifestBinding(RegisterKind.ConstantBuffer, 0, "hiz_cb0", 0, 0, false, TextureOverride.None, null),
                    new ManifestBinding(RegisterKind.Resource, 0, "hiz_depth", 0, 1, false, TextureOverride.Depth, null),
                    new ManifestBinding(RegisterKind.Sampler, 2, "hiz_s2", 0, 2, false, TextureOverride.None, null),
                    new ManifestBinding(RegisterKind.UnorderedAccess, 1, "hiz_out", 0, 3, false, TextureOverride.None, "r32float"),
                    new ManifestBinding(RegisterKind.ConstantBuffer, 12, "frame.cb12", 2, 0, true, TextureOverride.None, null),
                ],
                shader.Bindings);
        }

        [Fact]
        public void Parse_Defaults_AreDefaultNanPolicyAndNoBindings()
        {
            var shader = Assert.Single(Parse(OneShader()).Shaders);

            Assert.Equal(NanPolicy.Unspecified, shader.NanPolicy);
            Assert.Empty(shader.Bindings);
        }

        [Theory]
        [InlineData("""{"shaders":[],"extra":1}""")]
        [InlineData("""{"shaders":[{"name":"a","blob":1,"stage":"vertex","colour":"x"}]}""")]
        [InlineData("""{"shaders":[{"name":"a","blob":1,"stage":"vertex","bindings":[{"register":"cb0","name":"n","group":0,"binding":0,"nope":1}]}]}""")]
        [InlineData("""{"shaders":[],"shaders":[]}""")]
        public void Parse_UnknownOrDuplicateFields_Throw(string json)
        {
            Assert.Throws<ShaderManifestException>(() => Parse(json));
        }

        [Fact]
        public void Parse_DuplicateShaderNames_Throw()
        {
            var ex = Assert.Throws<ShaderManifestException>(() => Parse("""
                {"shaders":[{"name":"a","blob":1,"stage":"vertex"},{"name":"a","blob":2,"stage":"fragment"}]}
                """));

            Assert.Contains("duplicate shader name 'a'", ex.Message);
        }

        [Theory]
        [InlineData("cb")]
        [InlineData("x3")]
        [InlineData("cb-1")]
        [InlineData("t03")]
        [InlineData("CB0")]
        [InlineData("t99999999999")]
        public void Parse_BadRegister_Throws(string register)
        {
            Assert.Throws<ShaderManifestException>(() => Parse(OneShader(bindings: Binding(register))));
        }

        [Fact]
        public void Parse_SameRegisterBoundTwice_Throws()
        {
            string bindings = Binding("cb0", "a") + "," + Binding("cb0", "b").Replace("\"binding\":3", "\"binding\":4");

            Assert.Throws<ShaderManifestException>(() => Parse(OneShader(bindings: bindings)));
        }

        [Fact]
        public void Parse_TwoDeclaredBindingsOnOneSlot_Throw()
        {
            string bindings = Binding("cb0", "a") + "," + Binding("cb1", "b");

            Assert.Throws<ShaderManifestException>(() => Parse(OneShader(bindings: bindings)));
        }

        [Fact]
        public void Parse_ExternalBindingsMaySharedASlot()
        {
            string bindings = Binding("cb0", "frame.cb0", ",\"external\":true") + "," + Binding("cb1", "frame.cb1", ",\"external\":true");

            Assert.Equal(2, Assert.Single(Parse(OneShader(bindings: bindings)).Shaders).Bindings.Count);
        }

        [Theory]
        [InlineData("water.cb0")]
        [InlineData("a b")]
        [InlineData("1abc")]
        [InlineData("")]
        public void Parse_DeclaredBindingName_MustBeAPlainIdentifier(string name)
        {
            Assert.Throws<ShaderManifestException>(() => Parse(OneShader(bindings: Binding(name: name))));
        }

        [Theory]
        [InlineData("water_frame.cb12", true)]
        [InlineData("a.b.c", true)]
        [InlineData("plain", true)]
        [InlineData("a..b", false)]
        [InlineData(".a", false)]
        [InlineData("a.", false)]
        [InlineData("a.1b", false)]
        [InlineData("a-b.c", false)]
        public void Parse_ExternalBindingName_MayBeADottedMemberPath(string name, bool valid)
        {
            string json = OneShader(bindings: Binding(name: name, extra: ",\"external\":true"));

            if (valid)
                Assert.Equal(name, Assert.Single(Assert.Single(Parse(json).Shaders).Bindings).Name);
            else
                Assert.Throws<ShaderManifestException>(() => Parse(json));
        }

        [Theory]
        [InlineData("fragment", ShaderStage.Pixel)]
        [InlineData("vertex", ShaderStage.Vertex)]
        [InlineData("compute", ShaderStage.Compute)]
        public void Parse_Stage_MapsToShaderStage(string stage, ShaderStage expected)
        {
            var shader = Assert.Single(Parse($$"""{"shaders":[{"name":"a","blob":0,"stage":"{{stage}}"}]}""").Shaders);

            Assert.Equal(expected, shader.Stage);
        }

        [Theory]
        [InlineData("""{"shaders":[{"name":"a","blob":0,"stage":"geometry"}]}""")]
        [InlineData("""{"shaders":[{"name":"a","blob":-1,"stage":"vertex"}]}""")]
        [InlineData("""{"shaders":[{"name":"a","blob":1.5,"stage":"vertex"}]}""")]
        [InlineData("""{"shaders":[{"name":"a","blob":"1","stage":"vertex"}]}""")]
        [InlineData("""{"shaders":[{"name":"a","stage":"vertex"}]}""")]
        [InlineData("""{"shaders":[{"name":"__a","blob":1,"stage":"vertex"}]}""")]
        [InlineData("""{"shaders":[{"name":"a-b","blob":1,"stage":"vertex"}]}""")]
        [InlineData("""{"shaders":[{"name":"a","blob":1,"stage":"vertex","nan_policy":"strict"}]}""")]
        [InlineData("""{"shaders":{}}""")]
        [InlineData("[]")]
        [InlineData("{")]
        [InlineData("")]
        public void Parse_InvalidShaderEntries_Throw(string json)
        {
            Assert.Throws<ShaderManifestException>(() => Parse(json));
        }

        [Theory]
        [InlineData("t0", "uav_format", "r32float")]
        [InlineData("u0", "uav_format", "r8unknown")]
        [InlineData("cb0", "texture_override", "depth")]
        [InlineData("t0", "texture_override", "stencil")]
        public void Parse_MisplacedOrUnknownTypedOptions_Throw(string register, string field, string value)
        {
            Assert.Throws<ShaderManifestException>(() => Parse(OneShader(bindings: Binding(register, "n", $",\"{field}\":\"{value}\""))));
        }

        [Fact]
        public void ReadFile_MissingFile_ThrowsManifestException()
        {
            Assert.Throws<ShaderManifestException>(() => ShaderManifestReader.ReadFile(Path.Combine(Path.GetTempPath(), Guid.NewGuid().ToString("N"), "none.json")));
        }

        [Fact]
        public void CheckedInManifest_ParsesAndReferencesOnlySupportedFeatures()
        {
            var manifest = Parse(RepoFiles.ReadAllText(ShaderManifestReader.RepoRelativePath));

            Assert.NotNull(manifest.Shaders);
        }
    }
}
