using System.Numerics;
using System.Reflection;
using System.Text;
using System.Text.Json.Nodes;
using System.Text.RegularExpressions;
using AssetConverter;
using InstallerTests.Support;
using NiflySharp;
using NiflySharp.Blocks;
using NiflySharp.Structs;
using Xunit;

namespace InstallerTests.Nif
{
    // The extras are contracted with the engine's serde types (src/engine/src/assets/types/effect_material.rs).
    [Collection(StaticStateCollections.NifConverterState)]
    public class EffectMaterialExtrasTests
    {
        private const string EnginePath = "src/engine/src/assets/types/effect_material.rs";

        // Values of a vanilla barrel-fire mist card: decal flag, z-test, double sided, standard blend, greyscale texture without palette flags.
        internal static EffectShaderData MistShader() => new(
            Flags1: 2348810240, Flags2: 48, UvOffset: Vector2.Zero, UvScale: Vector2.One,
            SourceTexture: "textures\\Effects\\MistTileLight_d.dds", GreyscaleTexture: "textures\\Effects\\Gradients\\DustAlphaBasicGrad.dds",
            EnvTexture: "", NormalTexture: "", EnvMaskTexture: "", ReflectanceTexture: "", LightingTexture: "", EmitGradientTexture: "",
            ClampMode: 3, LightingInfluence: 255, FalloffStartAngle: 1f, FalloffStopAngle: 1f, FalloffStartOpacity: 0f, FalloffStopOpacity: 0f,
            RefractionPower: 0f, BaseColor: new Vector4(0f, 0f, 0f, 1f), BaseColorScale: 1f, SoftFalloffDepth: 100f, EnvironmentMapScale: 1f,
            EmittanceColor: Vector3.Zero);

        internal static readonly EffectAlphaState StandardBlend = EffectAlphaState.FromFlags(4333, 128);

        internal static string Deploy(string raw) => raw.Replace('\\', '/').ToLowerInvariant().Replace("textures/", "");

        private static EffectSelection Selected(uint vertex = 0x4, uint pixel = 0x10000004) => new(vertex, pixel, null);

        private static JsonObject Build(
            EffectShaderData? shader = null, EffectAlphaState? alpha = null, EffectMaterialParams? material = null,
            IReadOnlyList<ControllerTrack>? controllers = null) =>
            EffectMaterialExtras.Build(shader ?? MistShader(), alpha ?? StandardBlend, material, Selected(), controllers ?? [], Deploy);

        [Fact]
        public void Build_BarrelMistShape_MatchesTheGoldenExtras()
        {
            var extras = Build();

            const string golden = """
                {"v":1,"vs":"effect_vs_00000004","ps":"effect_ps_10000004","flags1":2348810240,"flags2":48,
                 "blend":{"mode":"alpha","src":"src_alpha","dst":"inv_src_alpha","alpha_test":false},
                 "z_test":true,"z_write":false,"double_sided":true,"clamp":"wrap_s_wrap_t",
                 "textures":{"source":"effects/misttilelight_d.dds","greyscale":"effects/gradients/dustalphabasicgrad.dds","normal":"","env":"","env_mask":"","reflectance":"","lighting":"","emit_gradient":""},
                 "base_color":[0,0,0,1],"base_color_scale":1,
                 "falloff":{"start_angle":1,"stop_angle":1,"start_opacity":0,"stop_opacity":0},
                 "soft_falloff_depth":10,"uv_offset":[0,0],"uv_scale":[1,1],"lighting_influence":1,"env_map_scale":1,
                 "emittance_color":[0,0,0],"refraction_power":0,"alpha":1,"controllers":[]}
                """;
            AssertJsonEqual(golden, extras);
        }

        [Fact]
        public void Build_AdditiveShapeWithAlphaTest_KeepsAdditiveDistinctFromAlphaBlendAndConvertsTheThreshold()
        {
            var alpha = EffectAlphaState.FromFlags(1 | 6 << 1 | 0 << 5 | 1 << 9 | 4 << 10, 128);

            var blend = Build(alpha: alpha)["blend"]!.AsObject();

            AssertJsonEqual("""{"mode":"add","src":"src_alpha","dst":"one","alpha_test":true,"test_func":"greater","test_ref":0.5019608}""", blend);
        }

        [Fact]
        public void Build_MultiplicativeShape_WritesTheStoredFactors()
        {
            var alpha = EffectAlphaState.FromFlags(1 | 4 << 1 | 1 << 5, 0);

            var blend = Build(alpha: alpha)["blend"]!.AsObject();

            AssertJsonEqual("""{"mode":"mult","src":"dst_color","dst":"zero","alpha_test":false}""", blend);
        }

        [Fact]
        public void Build_ShapeWithoutBlend_HasNoFactorsButKeepsAlphaTest()
        {
            var alpha = EffectAlphaState.FromFlags(1 << 9 | 4 << 10, 64);

            var blend = Build(alpha: alpha)["blend"]!.AsObject();

            AssertJsonEqual("""{"mode":"none","alpha_test":true,"test_func":"greater","test_ref":0.2509804}""", blend);
        }

        [Fact]
        public void Build_ShapeWithoutAlphaProperty_IsBlendNone()
        {
            var extras = EffectMaterialExtras.Build(MistShader(), null, null, Selected(), [], Deploy);

            AssertJsonEqual("""{"mode":"none","alpha_test":false}""", extras["blend"]!);
        }

        [Fact]
        public void Build_ShaderValues_AreConvertedToEngineUnitsAndFractions()
        {
            var shader = MistShader() with { LightingInfluence = 51, SoftFalloffDepth = 25f, UvOffset = new Vector2(0.25f, -0.5f), UvScale = new Vector2(2f, 3f), ClampMode = 1 };

            var extras = Build(shader);

            Assert.Equal(0.2f, extras["lighting_influence"]!.GetValue<float>(), 1e-6f);
            Assert.Equal(2.5f, extras["soft_falloff_depth"]!.GetValue<float>(), 1e-6f);
            AssertJsonEqual("[0.25,-0.5]", extras["uv_offset"]!);
            AssertJsonEqual("[2,3]", extras["uv_scale"]!);
            Assert.Equal("clamp_s_wrap_t", extras["clamp"]!.GetValue<string>());
        }

        [Fact]
        public void Build_FlagBits_DriveDepthAndCullState()
        {
            var shader = MistShader() with { Flags1 = 0, Flags2 = 1 };

            var extras = Build(shader);

            Assert.False(extras["z_test"]!.GetValue<bool>());
            Assert.True(extras["z_write"]!.GetValue<bool>());
            Assert.False(extras["double_sided"]!.GetValue<bool>());
        }

        private const uint EnvironmentPixel = 0x10080004;
        private const string Cube = @"textures\shared\cubemaps\sky.dds";
        private const string CubeMask = @"textures\shared\cubemaps\sky_m.dds";

        private static EffectShaderData WithEnvironmentTextures() => MistShader() with { EnvTexture = Cube, EnvMaskTexture = CubeMask };

        private static JsonObject BuildEnvironment(uint pixel, Func<string, bool> exists) =>
            EffectMaterialExtras.Build(WithEnvironmentTextures(), StandardBlend, null, Selected(0x4, pixel), [], Deploy, exists);

        [Fact]
        public void Build_EnvironmentPermutationWithAResolvingCube_CarriesCubeAndMask()
        {
            var textures = BuildEnvironment(EnvironmentPixel, _ => true)["textures"]!;

            Assert.Equal("shared/cubemaps/sky.dds", textures["env"]!.GetValue<string>());
            Assert.Equal("shared/cubemaps/sky_m.dds", textures["env_mask"]!.GetValue<string>());
        }

        [Fact]
        public void Build_EnvironmentPermutationWithAMissingCube_DropsTheCubeKeepsTheMaskAndCountsIt()
        {
            var textures = BuildEnvironment(EnvironmentPixel, _ => false)["textures"]!;

            Assert.Equal("", textures["env"]!.GetValue<string>());
            Assert.Equal("shared/cubemaps/sky_m.dds", textures["env_mask"]!.GetValue<string>());
            Assert.Contains(NifAnimationStats.EffectSummaryLines(), line => line.StartsWith(EffectCounters.EnvironmentCubeUnresolved, StringComparison.Ordinal));
        }

        [Fact]
        public void Build_EnvironmentPermutationWithAMissingMask_DropsOnlyTheMask()
        {
            var textures = BuildEnvironment(EnvironmentPixel, path => !path.Contains("_m."))["textures"]!;

            Assert.Equal("shared/cubemaps/sky.dds", textures["env"]!.GetValue<string>());
            Assert.Equal("", textures["env_mask"]!.GetValue<string>());
        }

        [Fact]
        public void Build_PermutationWithoutTheEnvironmentBit_CarriesNoCubeAndChecksNothing()
        {
            bool asked = false;
            var textures = BuildEnvironment(0x10000004, _ => asked = true)["textures"]!;

            Assert.Equal("", textures["env"]!.GetValue<string>());
            Assert.False(asked);
        }

        [Fact]
        public void Build_MaterialFile_ReplacesShaderFieldsAndTexturesAndNeedsNoAlphaProperty()
        {
            var bgem = new BGEM
            {
                BaseTexture = "textures\\fx\\Flame_d.dds",
                GrayscaleTexture = "textures\\fx\\Grad.dds",
                NormalTexture = "textures\\fx\\Flame_n.dds",
                AlphaBlendMode = BaseMaterialFile.AlphaBlendModeType.Additive,
                ZBufferTest = true,
                ZBufferWrite = false,
                TwoSided = true,
                TileU = true,
                TileV = false,
                BaseColor = 0xFF8000,
                BaseColorScale = 2.5f,
                SoftDepth = 50f,
                LightingInfluence = 0.5f,
                Alpha = 0.75f,
                UOffset = 0.5f,
                UScale = 4f,
                VScale = 4f,
                FalloffStartAngle = 0.1f,
                FalloffStopAngle = 0.9f,
                FalloffStartOpacity = 0.2f,
                FalloffStopOpacity = 0.8f,
                EnvironmentMappingMaskScale = 3f,
                EmittanceColor = 0x0000FF,
                RefractionPower = 0.25f,
            };

            var extras = EffectMaterialExtras.Build(MistShader(), null, EffectMaterialParams.FromBgem(bgem), Selected(0x4, 0x30000024), [], Deploy);

            AssertJsonEqual("""{"mode":"add","src":"src_alpha","dst":"one","alpha_test":false}""", extras["blend"]!);
            Assert.Equal("wrap_s_clamp_t", extras["clamp"]!.GetValue<string>());
            Assert.Equal("fx/flame_d.dds", extras["textures"]!["source"]!.GetValue<string>());
            Assert.Equal("fx/flame_n.dds", extras["textures"]!["normal"]!.GetValue<string>());
            Assert.Equal("", extras["textures"]!["env"]!.GetValue<string>());
            AssertJsonEqual("[1,0.5019608,0,1]", extras["base_color"]!);
            Assert.Equal(2.5f, extras["base_color_scale"]!.GetValue<float>());
            Assert.Equal(5f, extras["soft_falloff_depth"]!.GetValue<float>(), 1e-6f);
            Assert.Equal(0.5f, extras["lighting_influence"]!.GetValue<float>());
            Assert.Equal(0.75f, extras["alpha"]!.GetValue<float>());
            Assert.Equal(3f, extras["env_map_scale"]!.GetValue<float>());
            AssertJsonEqual("[0,0,1]", extras["emittance_color"]!);
            AssertJsonEqual("[0.5,0]", extras["uv_offset"]!);
            Assert.Equal(0.25f, extras["refraction_power"]!.GetValue<float>());
            Assert.Equal(0.9f, extras["falloff"]!["stop_angle"]!.GetValue<float>());
            Assert.False(extras["z_write"]!.GetValue<bool>());
            Assert.True(extras["double_sided"]!.GetValue<bool>());
            Assert.Equal(2348810240u, extras["flags1"]!.GetValue<uint>());
        }

        [Fact]
        public void Build_Controllers_ArePassedThroughWithScalarAndVectorKeys()
        {
            ControllerTrack[] tracks =
            [
                new("emissive_mult", "loop", 0f, 50f, 1f, 0f, [0f, 0.32f], [[2.6f], [2.9f]]),
                new("emissive_color", "clamp", 0f, 1f, 1f, 0f, [0f, 1f], [[1f, 0.5f, 0f], [0f, 0f, 1f]]),
            ];

            var controllers = Build(controllers: tracks)["controllers"]!.AsArray();

            AssertJsonEqual("""{"target":"emissive_mult","cycle":"loop","start":0,"stop":50,"frequency":1,"phase":0,"times":[0,0.32],"values":[2.6,2.9]}""", controllers[0]!);
            AssertJsonEqual("""{"target":"emissive_color","cycle":"clamp","start":0,"stop":1,"frequency":1,"phase":0,"times":[0,1],"values":[[1,0.5,0],[0,0,1]]}""", controllers[1]!);
        }

        [Theory]
        [InlineData(4)]
        [InlineData(255)]
        public void Build_ClampModeOutsideTheKnownRange_IsUnreadable(byte mode)
        {
            Assert.Throws<EffectShaderUnreadableException>(() => Build(MistShader() with { ClampMode = mode }));
        }

        [Fact]
        public void Build_BlendFactorOutsideTheKnownRange_IsUnreadable()
        {
            Assert.Throws<EffectShaderUnreadableException>(() => Build(alpha: EffectAlphaState.FromFlags(1 | 12 << 1, 0)));
        }

        [Fact]
        public void Build_NonFiniteParameter_IsUnreadableRatherThanWrittenAsNull()
        {
            Assert.Throws<EffectShaderUnreadableException>(() => Build(MistShader() with { BaseColorScale = float.NaN }));
        }

        [Fact]
        public void Build_HiddenSelection_IsRejected()
        {
            Assert.Throws<ArgumentException>(() => EffectMaterialExtras.Build(MistShader(), null, null, EffectSelection.Hidden(0, "skinned"), [], Deploy));
        }

        [Fact]
        public void Build_EveryEmittedFieldIsAnEngineFieldAndEveryEngineFieldIsEmitted()
        {
            var extras = Build();

            AssertFieldsMatchRustStruct(extras, "EffectMaterial", expectAll: true);
            AssertFieldsMatchRustStruct(extras["blend"]!.AsObject(), "Blend", expectAll: false);
            AssertFieldsMatchRustStruct(extras["textures"]!.AsObject(), "EffectTextures", expectAll: true);
            AssertFieldsMatchRustStruct(extras["falloff"]!.AsObject(), "Falloff", expectAll: true);
        }

        [Fact]
        public void Build_EveryVocabularyWordIsAnEngineEnumVariant()
        {
            AssertVocabulary("BlendFactor", EffectMaterialExtras.BlendFactors);
            AssertVocabulary("TestFunc", EffectMaterialExtras.TestFunctions);
            AssertVocabulary("ClampMode", EffectMaterialExtras.ClampModes);
        }

        [Fact]
        public void Reflected_ShaderFieldNames_ExistOnTheNiflySharpTypes()
        {
            AssertPrivateFields(typeof(BSEffectShaderProperty), EffectShaderData.ReflectedFieldNames);
            AssertPrivateFields(typeof(BSLightingShaderProperty), RefractionShaderData.ReflectedFieldNames);
        }

        [Fact]
        public void Read_ShaderWithUnsetTexturePaths_YieldsEmptyStrings()
        {
            var data = EffectShaderData.Read(EffectShaderFixture.Create(source: "textures\\a_d.dds"));

            Assert.Equal("textures\\a_d.dds", data.SourceTexture);
            Assert.Equal("", data.ReflectanceTexture);
            Assert.Equal("", data.EmitGradientTexture);
        }

        [Fact]
        public void Text_FieldOfATypeWithoutContent_IsRejectedInsteadOfReadAsEmpty()
        {
            Assert.Throws<InvalidDataException>(() => EffectShaderData.Text(new ShaderWithoutTextContent(), "_path"));
        }

        [Fact]
        public void Text_FieldThatIsNull_IsAnUnsetPath()
        {
            Assert.Equal("", EffectShaderData.Text(new ShaderWithoutTextContent(), "_unset"));
        }

        private sealed class ShaderWithoutTextContent
        {
#pragma warning disable CS0169, CS0414, CS0649, IDE0052 // read through reflection
            private readonly object? _path = new();
            private readonly object? _unset;
#pragma warning restore CS0169, CS0414, CS0649, IDE0052
        }

        [Fact]
        public void Read_BarrelMistValues_RoundTripThroughTheReflectedFields()
        {
            var data = EffectShaderData.Read(EffectShaderFixture.Create(
                flags1: 2348810240, flags2: 48, source: "textures\\Effects\\MistTileLight_d.dds", clamp: 3, lightingInfluence: 255,
                falloff: (1f, 1f, 0f, 0f), baseColor: new Color4(0f, 0f, 0f, 1f), baseColorScale: 1f, softFalloffDepth: 100f));

            Assert.Equal(MistShader() with { GreyscaleTexture = "" }, data);
        }

        [Fact]
        public void Refraction_Build_MatchesTheEngineStructAndCarriesStrengthTexturesAndTracks()
        {
            var shader = new RefractionShaderData(0x80008001u, 33, 0.04f, new Vector2(0.1f, 0f), Vector2.One, MeshHasVertexColors: true);
            ControllerTrack[] tracks = [new("u_offset", "loop", 0f, 4f, 1f, 0f, [0f, 4f], [[0f], [1f]])];

            var extras = RefractionMaterialExtras.Build(shader, "effects/smokevapor02tile_n.dds", "effects/smokevapor01tile_n.dds", tracks);

            AssertJsonEqual("""
                {"v":1,"strength":0.04,"normal_a":"effects/smokevapor02tile_n.dds","normal_b":"effects/smokevapor01tile_n.dds",
                 "uv_offset":[0.1,0],"uv_scale":[1,1],"vertex_colors":true,"double_sided":false,
                 "controllers":[{"target":"u_offset","cycle":"loop","start":0,"stop":4,"frequency":1,"phase":0,"times":[0,4],"values":[0,1]}]}
                """, extras);
            AssertFieldsMatchRustStruct(extras, "RefractionMaterial", expectAll: true);
        }

        [Theory]
        [InlineData(0x8000u, true)]
        [InlineData(0x80000001u, false)]
        public void Refraction_Read_ReturnsDataOnlyForShadersWithBit15OfTheFirstFlagWord(uint flags1, bool expected)
        {
            var shader = EffectShaderFixture.CreateRefraction(flags1, 0, 0.25f, -1);

            var data = RefractionShaderData.Read(shader, true);

            Assert.Equal(expected, data != null);
            if (expected)
                Assert.Equal(0.25f, data!.Strength);
        }

        [Fact]
        public void Refraction_VertexColours_NeedTheShaderFlagAndTheColourStream()
        {
            Assert.True(new RefractionShaderData(0, 1u << 5, 0f, Vector2.Zero, Vector2.One, true).VertexColors);
            Assert.False(new RefractionShaderData(0, 1u << 5, 0f, Vector2.Zero, Vector2.One, false).VertexColors);
            Assert.False(new RefractionShaderData(0, 0, 0f, Vector2.Zero, Vector2.One, true).VertexColors);
        }

        [Theory]
        [InlineData("", 0x02CC5D05u)]
        [InlineData("a", 0x550D7456u)]
        [InlineData("abc", 0x32D153FFu)]
        [InlineData("Nobody inspects the spammish repetition", 0xE2293B2Fu)]
        public void Xxh32_MatchesTheReferenceVectors(string text, uint expected)
        {
            Assert.Equal(expected, Xxh32.Hash(Encoding.UTF8.GetBytes(text)));
        }

        [Fact]
        public void KeyOf_EqualExtrasShareAKeyAndDifferingParametersDoNot()
        {
            uint key = GltfMaterialExtras.KeyOf(Build());

            Assert.Equal(key, GltfMaterialExtras.KeyOf(Build()));
            Assert.NotEqual(key, GltfMaterialExtras.KeyOf(Build(MistShader() with { BaseColorScale = 2f })));
            Assert.NotEqual(key, GltfMaterialExtras.KeyOf(Build(alpha: EffectAlphaState.FromFlags(1 | 6 << 1, 0))));
        }

        private static void AssertJsonEqual(string expected, JsonNode actual)
        {
            var parsed = JsonNode.Parse(expected)!;
            Assert.True(JsonNode.DeepEquals(parsed, actual), $"expected {parsed.ToJsonString()}{Environment.NewLine}actual   {actual.ToJsonString()}");
        }

        private static List<string> RustFields(string structName)
        {
            string body = RepoFiles.ExtractBlock(EnginePath, $@"pub struct {structName} \{{(.*?)\n\}}", $"struct {structName}");
            return Regex.Matches(body, @"pub (\w+):").Select(m => m.Groups[1].Value).ToList();
        }

        private static void AssertFieldsMatchRustStruct(JsonObject extras, string structName, bool expectAll)
        {
            var rust = RustFields(structName);
            var emitted = extras.Select(p => p.Key).ToList();
            Assert.Empty(emitted.Except(rust));
            if (expectAll) Assert.Empty(rust.Except(emitted));
        }

        private static void AssertVocabulary(string enumName, IEnumerable<string> words)
        {
            string body = RepoFiles.ExtractBlock(EnginePath, $@"pub enum {enumName} \{{(.*?)\n\}}", $"enum {enumName}");
            var variants = Regex.Matches(body, @"^\s+(\w+),", RegexOptions.Multiline)
                .Select(m => Regex.Replace(m.Groups[1].Value, "(?<!^)([A-Z])", "_$1").ToLowerInvariant()).ToHashSet();
            Assert.Equal(variants.Order(), words.Order());
        }

        private static void AssertPrivateFields(Type type, IEnumerable<string> names)
        {
            foreach (string name in names)
                Assert.True(type.GetField(name, BindingFlags.NonPublic | BindingFlags.Instance) != null, $"{type.Name} has no private field {name}");
        }
    }

    /// <summary>Builds NiflySharp shader blocks; their data lives in private fields, so the fixtures set them by reflection.</summary>
    internal static class EffectShaderFixture
    {
        public static BSEffectShaderProperty Create(
            uint flags1 = 0, uint flags2 = 0, string? source = null, string? greyscale = null, byte clamp = 3, byte lightingInfluence = 255,
            (float StartAngle, float StopAngle, float StartOpacity, float StopOpacity)? falloff = null, Color4? baseColor = null,
            float baseColorScale = 1f, float softFalloffDepth = 100f)
        {
            var shader = new BSEffectShaderProperty();
            Set(shader, "_shaderFlags1_F4SPF1", Enum.ToObject(Field(shader, "_shaderFlags1_F4SPF1").FieldType, flags1));
            Set(shader, "_shaderFlags2_F4SPF2", Enum.ToObject(Field(shader, "_shaderFlags2_F4SPF2").FieldType, flags2));
            Set(shader, "_uVScale", new TexCoord(1f, 1f));
            if (source != null) Set(shader, "_sourceTexture", new NiString4(source, false));
            if (greyscale != null) Set(shader, "_greyscaleTexture", new NiString4(greyscale, false));
            Set(shader, "_textureClampMode", clamp);
            Set(shader, "_lightingInfluence", lightingInfluence);
            var (startAngle, stopAngle, startOpacity, stopOpacity) = falloff ?? (1f, 1f, 0f, 0f);
            Set(shader, "_falloffStartAngle", startAngle);
            Set(shader, "_falloffStopAngle", stopAngle);
            Set(shader, "_falloffStartOpacity", startOpacity);
            Set(shader, "_falloffStopOpacity", stopOpacity);
            Set(shader, "_baseColor", baseColor ?? new Color4(1f, 1f, 1f, 1f));
            Set(shader, "_baseColorScale", baseColorScale);
            Set(shader, "_softFalloffDepth", softFalloffDepth);
            Set(shader, "_environmentMapScale_fl", 1f);
            return shader;
        }

        public static BSLightingShaderProperty CreateRefraction(uint flags1, uint flags2, float strength, int textureSet)
        {
            var shader = new BSLightingShaderProperty();
            Set(shader, "_textureSet", new NiBlockRef<BSShaderTextureSet>(textureSet));
            Set(shader, "_shaderFlags1_F4SPF1", Enum.ToObject(Field(shader, "_shaderFlags1_F4SPF1").FieldType, flags1));
            Set(shader, "_shaderFlags2_F4SPF2", Enum.ToObject(Field(shader, "_shaderFlags2_F4SPF2").FieldType, flags2));
            Set(shader, "_refractionStrength", strength);
            Set(shader, "_uVScale", new TexCoord(1f, 1f));
            return shader;
        }

        private static FieldInfo Field(object target, string name) =>
            target.GetType().GetField(name, BindingFlags.NonPublic | BindingFlags.Instance)
            ?? throw new InvalidOperationException($"NiflySharp has no field {name}");

        private static void Set(object target, string name, object value) => Field(target, name).SetValue(target, value);
    }
}
