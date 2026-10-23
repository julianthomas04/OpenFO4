using System.Globalization;
using System.Text.RegularExpressions;
using AssetConverter;
using InstallerTests.Support;
using Xunit;

namespace InstallerTests
{
    // Grammar: the MaterialNameCodec class summary. The only decoder is the engine's
    // (src/engine/src/assets/types/material_name.rs), so decode-side behaviour is
    // covered by cross-checking the producer's vocabulary against that source.
    public class MaterialNameCodecTests
    {
        [Fact]
        public void Encode_EverySegmentEnabled_EmitsSegmentsInDocumentedOrder()
        {
            string name = MaterialNameCodec.Encode(FullyEnabledParts());

            Assert.Equal(
                "d.dds|n.dds|blend|0.50|fx|refr|eff:0a1b2c3d|decal|spec:s.dds|glow:g.dds|sc:00FF10|sm:1.5|sg:0.25|fp:5|em:A0B0C0|emm:2|ts|src:mat/key",
                name);
        }

        [Fact]
        public void Encode_PlainTexturedMaterial_IsJustDiffuseAndNormal()
        {
            string name = MaterialNameCodec.Encode(Parts());

            Assert.Equal("d.dds|n.dds", name);
        }

        [Fact]
        public void Encode_EmptyNormal_KeepsTheEmptySegmentSoPositionsStayStable()
        {
            string name = MaterialNameCodec.Encode(Parts() with { Normal = "", TwoSided = true });

            Assert.Equal("d.dds||ts", name);
        }

        [Fact]
        public void Encode_AlphaTestToken_IsEmittedVerbatim()
        {
            string name = MaterialNameCodec.Encode(Parts() with { AlphaToken = "test:128" });

            Assert.Equal("d.dds|n.dds|test:128", name);
        }

        [Fact]
        public void Encode_SpecularEnabledWithoutTextures_EmitsOnlyColourAndScalars()
        {
            var lighting = LightingParams.None with { SpecularEnabled = true };

            string name = MaterialNameCodec.Encode(Parts() with { Lighting = lighting });

            Assert.Equal("d.dds|n.dds|sc:FFFFFF|sm:1|sg:1|fp:5", name);
        }

        [Fact]
        public void Encode_SpecularDisabledButTexturesSet_OmitsSpecularTokensAndKeepsGlow()
        {
            var lighting = LightingParams.None with { SpecularTexture = "s.dds", GlowTexture = "g.dds" };

            string name = MaterialNameCodec.Encode(Parts() with { Lighting = lighting });

            Assert.Equal("d.dds|n.dds|glow:g.dds", name);
        }

        [Theory]
        [InlineData(false, 0xFFFFFF, 1f)]
        [InlineData(true, 0x000000, 1f)]
        [InlineData(true, 0xFFFFFF, 0f)]
        public void Encode_EmissiveDisabledBlackOrZeroMultiplier_OmitsEmissiveTokens(bool enabled, int color, float mult)
        {
            var lighting = LightingParams.None with { EmissiveEnabled = enabled, EmissiveColor = color, EmissiveMult = mult };

            string name = MaterialNameCodec.Encode(Parts() with { Lighting = lighting });

            Assert.Equal("d.dds|n.dds", name);
        }

        [Theory]
        [InlineData(0x00000A, "00000A")]
        [InlineData(0xABCDEF, "ABCDEF")]
        [InlineData(0xFF0000, "FF0000")]
        public void Encode_ColourToken_IsSixUppercaseHexDigits(int color, string expectedHex)
        {
            var lighting = LightingParams.None with { SpecularEnabled = true, SpecularColor = color };

            string name = MaterialNameCodec.Encode(Parts() with { Lighting = lighting });

            Assert.Contains($"|sc:{expectedHex}|", name);
        }

        [Theory]
        [InlineData(1f, "1")]
        [InlineData(0.5f, "0.5")]
        [InlineData(-0.5f, "-0.5")]
        [InlineData(0.12345f, "0.1235")]
        [InlineData(12.0f, "12")]
        public void Encode_ScalarToken_UsesAtMostFourDecimals(float value, string expected)
        {
            var lighting = LightingParams.None with { SpecularEnabled = true, SpecularMult = value };

            string name = MaterialNameCodec.Encode(Parts() with { Lighting = lighting });

            Assert.Contains($"|sm:{expected}|", name);
        }

        [Fact]
        public void Encode_GermanCulture_StillWritesDecimalPoints()
        {
            var previous = CultureInfo.CurrentCulture;
            CultureInfo.CurrentCulture = new CultureInfo("de-DE");
            try
            {
                var lighting = LightingParams.None with { SpecularEnabled = true, SpecularMult = 1.5f };

                string name = MaterialNameCodec.Encode(Parts() with { Lighting = lighting });

                Assert.Contains("|sm:1.5|", name);
            }
            finally
            {
                CultureInfo.CurrentCulture = previous;
            }
        }

        [Fact]
        public void Encode_WaterShader_EmitsWaterTokenAfterDecalAndBeforeLighting()
        {
            var water = new WaterShaderParams(WaterFlags: 452, ShaderFlags1: 0x80000008, ShaderFlags2: 1, UvOffsetU: 0.5f, UvOffsetV: -0.25f, UvScaleU: 1f, UvScaleV: 2.5f);
            var lighting = LightingParams.None with { GlowTexture = "g.dds" };

            string name = MaterialNameCodec.Encode(Parts() with { Effect = true, Decal = true, Water = water, Lighting = lighting });

            Assert.Equal("d.dds|n.dds|fx|decal|water:452:80000008:00000001:0.5,-0.25:1,2.5|glow:g.dds", name);
        }

        [Fact]
        public void WaterToken_IsCultureInvariant()
        {
            var previous = CultureInfo.CurrentCulture;
            CultureInfo.CurrentCulture = new CultureInfo("de-DE");
            try
            {
                string token = new WaterShaderParams(0, 0, 0, 0.5f, 0f, 1.5f, 1f).ToToken();

                Assert.Equal("water:0:00000000:00000000:0.5,0:1.5,1", token);
            }
            finally
            {
                CultureInfo.CurrentCulture = previous;
            }
        }

        [Fact]
        public void Encode_ExtrasKey_IsEightLowercaseHexDigitsEvenWhenSmall()
        {
            string name = MaterialNameCodec.Encode(Parts() with { Effect = true, ExtrasKey = 0xABu });

            Assert.Equal("d.dds|n.dds|fx|eff:000000ab", name);
        }

        [Fact]
        public void Encode_RefractionShape_CarriesTheRefrTokenWithoutFx()
        {
            string name = MaterialNameCodec.Encode(Parts() with { Refraction = true, ExtrasKey = 0xFFFFFFFFu });

            Assert.Equal("d.dds|n.dds|refr|eff:ffffffff", name);
        }

        [Fact]
        public void Encode_NullSourceKey_OmitsSrcTokenWhileEmptyKeyEmitsIt()
        {
            string withoutKey = MaterialNameCodec.Encode(Parts() with { SourceKey = null });
            string withEmptyKey = MaterialNameCodec.Encode(Parts() with { SourceKey = "" });

            Assert.Equal("d.dds|n.dds", withoutKey);
            Assert.Equal("d.dds|n.dds|src:", withEmptyKey);
        }

        [Fact]
        public void ForDefinition_MaterialDefinition_CopiesFlagsKeyAndAlphaToken()
        {
            var def = new MaterialDef(
                Key: "materials/a/b.bgsm", Diffuse: "ignored.dds", Normal: "ignored_n.dds", AlphaToken: "blend",
                TwoSided: true, Decal: true, IsEffect: true, IsPalette: false, Lut: "", Row: 0f,
                Lighting: LightingParams.None);

            string name = MaterialNameCodec.ForDefinition(def, "diff.dds", "norm.dds");

            Assert.Equal("diff.dds|norm.dds|blend|fx|decal|ts|src:materials/a/b.bgsm", name);
        }

        [Fact]
        public void CleanLighting_RawTexturePaths_AreDeployedFormWithoutTexturesPrefix()
        {
            var raw = LightingParams.None with { SpecularTexture = "Textures\\Foo\\Bar_s.dds", GlowTexture = "" };

            var clean = MaterialNameCodec.CleanLighting(raw);

            Assert.Equal("foo/bar_s.dds", clean.SpecularTexture);
            Assert.Equal("", clean.GlowTexture);
        }

        // CleanLighting reads NifConverter's static texture index. No test populates it (empty index = no path repair),
        // so these tests stay deterministic under xUnit's cross-class parallelism; a test that builds the index needs a collection.

        // Contract with the engine parser: every key the producer can write must be a token it knows,
        // otherwise the engine rejects the whole material name (UnknownToken).
        [Fact]
        public void Encode_EveryProducedToken_IsKnownToTheEngineParser()
        {
            string isTokenBody = RepoFiles.ExtractBlock(
                "src/engine/src/assets/types/material_name.rs", @"(fn is_token.*?)fn texture_path", "fn is_token");
            var engineTokens = Regex.Matches(isTokenBody, "\"(\\w+)\"").Select(m => m.Groups[1].Value).ToHashSet();
            Assert.True(engineTokens.Count > 5, $"parsed only {engineTokens.Count} tokens from the engine's is_token; update the test parser");

            var producedTokens = MaterialNameCodec.Encode(FullyEnabledParts()).Split('|')
                .Skip(2)
                .Where(segment => !double.TryParse(segment, NumberStyles.Float, CultureInfo.InvariantCulture, out _))
                .Select(segment => segment.Split(':')[0])
                .ToList();

            var unknown = producedTokens.Where(token => !engineTokens.Contains(token)).ToList();
            Assert.True(unknown.Count == 0, $"producer writes tokens the engine rejects: {string.Join(", ", unknown)}");
            Assert.Equal(15, producedTokens.Count);
        }

        [Fact]
        public void Encode_CarGlassEffect_EmitsExampleAInGrammarOrder()
        {
            var effect = new EffectParams(EffectParams.BlendAlpha, 0x1A2B3C, 0.1f, null, 0.98f, "shared/glass01grad_d.dds", true, false);
            var env = new EnvironmentParams("shared/cubemaps/mipblur_defaultoutside1.dds", "shared/glasstile01_g.dds", 0.2f);

            string name = MaterialNameCodec.Encode(Parts() with
            {
                Diffuse = "shared/glasstile01_d.dds",
                Normal = "",
                AlphaToken = "blend|1.01",
                Effect = true,
                TwoSided = true,
                SourceKey = "materials/vehicles/car_glass01.bgem",
                EffectFields = effect,
                Env = env,
            });

            Assert.Equal(
                "shared/glasstile01_d.dds||blend|1.01|fx|bc:1A2B3C|bs:0.1|li:0.98|pal:c:shared/glass01grad_d.dds"
                + "|env:shared/cubemaps/mipblur_defaultoutside1.dds|envk:shared/glasstile01_g.dds|envs:0.2|ts|src:materials/vehicles/car_glass01.bgem",
                name);
        }

        [Fact]
        public void Encode_FalloffGlassWithoutTextures_EmitsExampleBWithEmptyLeadingSegments()
        {
            var effect = new EffectParams(EffectParams.BlendAlpha, 0, 1f, new FalloffParams(1f, 0.5f, 1f, 0.25f, false), null, "", false, false);
            var env = new EnvironmentParams("shared/cubemaps/bronze_e.dds", "", 1.5f);

            string name = MaterialNameCodec.Encode(Parts() with
            {
                Diffuse = "",
                Normal = "",
                AlphaToken = "blend|0.68",
                Effect = true,
                SourceKey = "materials/x/nukacolaglass.bgem",
                EffectFields = effect,
                Env = env,
            });

            Assert.Equal(
                "||blend|0.68|fx|bc:000000|bs:1|fo:1,0.5,1,0.25,0|env:shared/cubemaps/bronze_e.dds|envs:1.5|src:materials/x/nukacolaglass.bgem",
                name);
        }

        [Fact]
        public void Encode_LitWindowWithEnvironment_PutsEnvTokensBeforeSpecular()
        {
            var lighting = LightingParams.None with { SpecularEnabled = true };
            var env = new EnvironmentParams("shared/cubemaps/outsideoldtownreflectcube_e.dds", "", 1f);

            string name = MaterialNameCodec.Encode(Parts() with { AlphaToken = "test:128", Lighting = lighting, TwoSided = true, Env = env });

            Assert.Equal("d.dds|n.dds|test:128|env:shared/cubemaps/outsideoldtownreflectcube_e.dds|envs:1|sc:FFFFFF|sm:1|sg:1|fp:5|ts", name);
        }

        [Theory]
        [InlineData(EffectParams.BlendAdd, "blend|fx|bm:add|bc:FFFFFF|bs:1")]
        [InlineData(EffectParams.BlendMul, "blend|fx|bm:mul|bc:FFFFFF|bs:1")]
        [InlineData(EffectParams.BlendAlpha, "blend|fx|bc:FFFFFF|bs:1")]
        public void Encode_BlendMode_IsAnExplicitTokenOnlyWhenNotStandardAlpha(string mode, string expectedTail)
        {
            var effect = new EffectParams(mode, 0xFFFFFF, 1f, null, null, "", false, false);

            string name = MaterialNameCodec.Encode(Parts() with { AlphaToken = "blend", Effect = true, EffectFields = effect });

            Assert.Equal($"d.dds|n.dds|{expectedTail}", name);
        }

        [Theory]
        [InlineData(true, false, "pal:c:p.dds")]
        [InlineData(false, true, "pal:a:p.dds")]
        [InlineData(true, true, "pal:ca:p.dds")]
        public void Encode_Palette_CarriesTheFlagsThatApply(bool color, bool alpha, string expected)
        {
            var effect = new EffectParams(EffectParams.BlendAlpha, 0xFFFFFF, 1f, null, null, "p.dds", color, alpha);

            string name = MaterialNameCodec.Encode(Parts() with { EffectFields = effect });

            Assert.Contains($"|{expected}", name);
        }

        [Theory]
        [InlineData("p.dds", false)]
        [InlineData("", true)]
        public void Encode_PaletteTextureWithoutFlagsOrFlagsWithoutTexture_EmitsNoPaletteToken(string texture, bool flags)
        {
            var effect = new EffectParams(EffectParams.BlendAlpha, 0xFFFFFF, 1f, null, null, texture, flags, flags);

            string name = MaterialNameCodec.Encode(Parts() with { EffectFields = effect });

            Assert.DoesNotContain("pal:", name);
        }

        [Fact]
        public void Encode_FalloffAndLightingInfluence_FoldTinyNegativesIntoZero()
        {
            var effect = new EffectParams(EffectParams.BlendAlpha, 0xFFFFFF, 1f, new FalloffParams(-4.371139E-08f, -0.985f, 1f, 0f, true), 0f, "", false, false);

            string name = MaterialNameCodec.Encode(Parts() with { EffectFields = effect });

            Assert.EndsWith("|bs:1|fo:0,-0.985,1,0,1|li:0", name);
        }

        [Fact]
        public void Encode_EffectTokens_AreCultureInvariant()
        {
            var previous = CultureInfo.CurrentCulture;
            CultureInfo.CurrentCulture = new CultureInfo("de-DE");
            try
            {
                var effect = new EffectParams(EffectParams.BlendAlpha, 0xFFFFFF, 0.5f, new FalloffParams(0.5f, 0.25f, 1f, 0f, false), 0.75f, "", false, false);

                string name = MaterialNameCodec.Encode(Parts() with { EffectFields = effect, Env = new EnvironmentParams("e.dds", "", 0.2f) });

                Assert.EndsWith("|bs:0.5|fo:0.5,0.25,1,0,0|li:0.75|env:e.dds|envs:0.2", name);
            }
            finally
            {
                CultureInfo.CurrentCulture = previous;
            }
        }

        [Fact]
        public void Encode_EnvironmentPathContainingColon_IsWrittenVerbatim()
        {
            string name = MaterialNameCodec.Encode(Parts() with { Env = new EnvironmentParams("a:b.dds", "", 1f) });

            Assert.EndsWith("|env:a:b.dds|envs:1", name);
        }

        [Fact]
        public void ForDefinition_EffectAndEnvironment_AreForwardedWithDeployedTexturePaths()
        {
            var effect = EffectMaterialParams.FromBgem(EffectParamsTests.ParseBgem(new BgemFields
            {
                Base = new MaterialBaseFields { BlendA = 1, BlendB = 4, BlendC = 1, GrayscaleToPalette = true },
                Greyscale = "Textures\\Shared\\Grad_d.dds",
                EffectLighting = true,
                LightingInfluence = 1f,
                BaseColor = (16 / 255f, 32 / 255f, 48 / 255f),
                BaseColorScale = 2f,
            }));
            var env = new EnvironmentParams("Textures\\Shared\\Cubemaps\\Bronze_e.dds", "Textures\\Shared\\Mask_g.dds", 0.3f);
            var def = new MaterialDef(
                Key: "materials/a/b.bgem", Diffuse: "", Normal: "", AlphaToken: "blend", TwoSided: false, Decal: false, IsEffect: true,
                IsPalette: false, Lut: "", Row: 0f, Lighting: LightingParams.None, Effect: effect, Env: env);

            string name = MaterialNameCodec.ForDefinition(def, "", "");

            Assert.Equal(
                "||blend|fx|bm:mul|bc:102030|bs:2|li:1|pal:c:shared/grad_d.dds|env:shared/cubemaps/bronze_e.dds|envk:shared/mask_g.dds|envs:0.3|src:materials/a/b.bgem",
                name);
        }

        // New tokens must be known to the engine parser, otherwise it rejects the whole name.
        // Fails until the engine's is_token lists the effect and environment keys.
        [Fact]
        public void Encode_EffectAndEnvironmentTokens_AreKnownToTheEngineParser()
        {
            string isTokenBody = RepoFiles.ExtractBlock(
                "src/engine/src/assets/types/material_name.rs", @"(fn is_token.*?)fn texture_path", "fn is_token");
            var engineTokens = Regex.Matches(isTokenBody, "\"(\\w+)\"").Select(m => m.Groups[1].Value).ToHashSet();
            var effect = new EffectParams(EffectParams.BlendAdd, 0xFFFFFF, 1f, new FalloffParams(1f, 0f, 1f, 0f, true), 1f, "p.dds", true, true);
            var env = new EnvironmentParams("e.dds", "m.dds", 1f);

            var produced = MaterialNameCodec.Encode(Parts() with { EffectFields = effect, Env = env }).Split('|')
                .Skip(2).Select(segment => segment.Split(':')[0]).ToList();

            var unknown = produced.Where(token => !engineTokens.Contains(token)).ToList();
            Assert.True(unknown.Count == 0, $"producer writes tokens the engine rejects: {string.Join(", ", unknown)}");
            Assert.Equal(9, produced.Count);
        }

        private static MaterialNameParts Parts() => new(
            Diffuse: "d.dds", Normal: "n.dds", AlphaToken: "", Effect: false, Decal: false,
            Lighting: LightingParams.None, TwoSided: false, SourceKey: null);

        private static MaterialNameParts FullyEnabledParts()
        {
            var lighting = new LightingParams(
                SpecularEnabled: true, SpecularColor: 0x00FF10, SpecularMult: 1.5f, Smoothness: 0.25f, FresnelPower: 5f,
                EmissiveEnabled: true, EmissiveColor: 0xA0B0C0, EmissiveMult: 2f,
                SpecularTexture: "s.dds", GlowTexture: "g.dds");
            return Parts() with { AlphaToken = "blend|0.50", Effect = true, Decal = true, Lighting = lighting, TwoSided = true, SourceKey = "mat/key", Refraction = true, ExtrasKey = 0x0A1B2C3Du };
        }
    }
}
