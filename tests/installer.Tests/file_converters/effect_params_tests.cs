using AssetConverter;
using InstallerTests.Support;
using Xunit;

namespace InstallerTests
{
    // BGEM -> EffectParams / EnvironmentParams and the alpha token rules of MaterialCensus.GetAlphaToken.
    public class EffectParamsTests
    {
        [Theory]
        [InlineData(1.01f, "blend|1.01")]
        [InlineData(6.68f, "blend|6.68")]
        [InlineData(0.68f, "blend|0.68")]
        [InlineData(1.0f, "blend")]
        [InlineData(1.0005f, "blend")]
        public void GetAlphaToken_BlendedBgem_WritesTheAlphaMultiplierWheneverItDiffersFromOne(float alpha, string expected)
        {
            var bgem = ParseBgem(new BgemFields { Base = new MaterialBaseFields { Alpha = alpha, BlendA = 1, BlendB = 6, BlendC = 7 } });

            Assert.Equal(expected, MaterialCensus.GetAlphaToken(bgem));
        }

        [Fact]
        public void GetAlphaToken_UnblendedBgemWithAlphaAboveOne_DoesNotBlend()
        {
            var bgem = ParseBgem(new BgemFields { Base = new MaterialBaseFields { Alpha = 1.5f } });

            Assert.Equal("", MaterialCensus.GetAlphaToken(bgem));
        }

        [Fact]
        public void GetAlphaToken_UnblendedBgemBelowOne_BlendsWithTheScale()
        {
            var bgem = ParseBgem(new BgemFields { Base = new MaterialBaseFields { Alpha = 0.5f } });

            Assert.Equal("blend|0.5", MaterialCensus.GetAlphaToken(bgem));
        }

        [Theory]
        [InlineData(0.5f, 0.5f, 0.5f, 0x808080)]
        [InlineData(0.999f, 0.001f, 0.2f, 0xFF0033)]
        [InlineData(1.7f, -0.3f, 1f, 0xFF00FF)]
        public void FromBgem_BaseColor_IsRoundedAndClampedToBytes(float r, float g, float b, int expected)
        {
            var bgem = ParseBgem(new BgemFields { BaseColor = (r, g, b) });

            Assert.Equal(expected, EffectParams.FromMaterial(EffectMaterialParams.FromBgem(bgem)).BaseColor);
        }

        [Fact]
        public void GetAlphaToken_BgsmWithAlphaAboveOne_KeepsTheOpaqueBehaviour()
        {
            var bgsm = ParseBgsm(new BgsmFields { Base = new MaterialBaseFields { Alpha = 1.01f } });

            Assert.Equal("", MaterialCensus.GetAlphaToken(bgsm));
        }

        [Fact]
        public void GetAlphaToken_BgsmBelowOne_UsesTwoDecimals()
        {
            var bgsm = ParseBgsm(new BgsmFields { Base = new MaterialBaseFields { Alpha = 0.5f } });

            Assert.Equal("blend|0.50", MaterialCensus.GetAlphaToken(bgsm));
        }

        [Fact]
        public void FromBgem_EffectBlock_IsCopiedWithBlendModeFalloffLightingAndPalette()
        {
            var bgem = ParseBgem(new BgemFields
            {
                Base = new MaterialBaseFields { BlendA = 1, BlendB = 6, BlendC = 0, GrayscaleToPalette = true },
                Greyscale = "textures\\shared\\grad_d.dds",
                EffectLighting = true,
                LightingInfluence = 0.98f,
                Falloff = true,
                FalloffColor = true,
                FalloffStartAngle = 1f,
                FalloffStopAngle = 0.5f,
                FalloffStartOpacity = 0.25f,
                FalloffStopOpacity = 0.75f,
                GrayscaleToPaletteAlpha = true,
                BaseColor = (1f, 0f, 0f),
                BaseColorScale = 2f,
            });

            var effect = EffectParams.FromMaterial(EffectMaterialParams.FromBgem(bgem));

            Assert.Equal(EffectParams.BlendAdd, effect.BlendMode);
            Assert.Equal(0xFF0000, effect.BaseColor);
            Assert.Equal(2f, effect.BaseColorScale);
            Assert.Equal(new FalloffParams(1f, 0.5f, 0.25f, 0.75f, ScalesColor: true), effect.Falloff);
            Assert.Equal(0.98f, effect.LightingInfluence);
            Assert.Equal("textures\\shared\\grad_d.dds", effect.PaletteTexture);
            Assert.True(effect.PaletteColor);
            Assert.True(effect.PaletteAlpha);
        }

        [Fact]
        public void FromBgem_FalloffAndEffectLightingOff_LeavesBothNull()
        {
            var effect = EffectParams.FromMaterial(EffectMaterialParams.FromBgem(ParseBgem(new BgemFields { LightingInfluence = 0.5f })));

            Assert.Null(effect.Falloff);
            Assert.Null(effect.LightingInfluence);
            Assert.Equal(EffectParams.BlendAlpha, effect.BlendMode);
        }

        [Fact]
        public void FromBgem_MultiplicativeBlend_IsMul()
        {
            var bgem = ParseBgem(new BgemFields { Base = new MaterialBaseFields { BlendA = 1, BlendB = 4, BlendC = 1 } });

            Assert.Equal(EffectParams.BlendMul, EffectParams.FromMaterial(EffectMaterialParams.FromBgem(bgem)).BlendMode);
        }

        [Fact]
        public void FromBgem_LightingInfluenceAboveOne_IsClampedToOne()
        {
            var bgem = ParseBgem(new BgemFields { EffectLighting = true, LightingInfluence = 1.5f });

            Assert.Equal(1f, EffectParams.FromMaterial(EffectMaterialParams.FromBgem(bgem)).LightingInfluence);
        }

        [Fact]
        public void HasPalette_NeedsATextureAndAtLeastOneFlag()
        {
            var none = new EffectParams(EffectParams.BlendAlpha, 0xFFFFFF, 1f, null, null, "", true, true);
            var noFlag = none with { PaletteTexture = "p.dds", PaletteColor = false, PaletteAlpha = false };
            var withBoth = none with { PaletteTexture = "p.dds" };

            Assert.False(none.HasPalette);
            Assert.False(noFlag.HasPalette);
            Assert.True(withBoth.HasPalette);
        }

        [Fact]
        public void Resolve_BgemEnvironmentWithExistingCubeAndMask_KeepsBothAndTheScale()
        {
            var bgem = ParseBgem(new BgemFields
            {
                Base = new MaterialBaseFields { EnvironmentMapping = true, EnvironmentMappingMaskScale = 0.2f },
                Envmap = "textures\\shared\\cubemaps\\Bronze_e.dds",
                EnvmapMask = "textures\\shared\\mask_g.dds",
            });
            var existing = new HashSet<string> { "shared/cubemaps/bronze_e.dds", "shared/mask_g.dds" };

            var env = EnvironmentParams.Resolve(bgem.EnvironmentMapping, bgem.EnvmapTexture, bgem.EnvmapMaskTexture,
                bgem.EnvironmentMappingMaskScale, existing.Contains, out bool cubeMissing);

            Assert.False(cubeMissing);
            Assert.Equal(new EnvironmentParams("textures\\shared\\cubemaps\\Bronze_e.dds", "textures\\shared\\mask_g.dds", 0.2f), env);
        }

        [Fact]
        public void Resolve_CubeNotInTextureIndex_ReturnsNullAndFlagsItAsMissing()
        {
            var env = EnvironmentParams.Resolve(true, "cube.dds", "mask.dds", 1f, _ => false, out bool cubeMissing);

            Assert.Null(env);
            Assert.True(cubeMissing);
        }

        [Fact]
        public void Resolve_MaskNotInTextureIndex_DropsOnlyTheMask()
        {
            var env = EnvironmentParams.Resolve(true, "cube.dds", "mask.dds", 3f, path => path == "cube.dds", out bool cubeMissing);

            Assert.False(cubeMissing);
            Assert.Equal(new EnvironmentParams("cube.dds", "", 3f), env);
        }

        [Theory]
        [InlineData(false, "cube.dds")]
        [InlineData(true, "")]
        [InlineData(true, null)]
        public void Resolve_MappingOffOrNoCubePath_IsNullWithoutCountingAMissingCube(bool enabled, string? cube)
        {
            var env = EnvironmentParams.Resolve(enabled, cube, "mask.dds", 1f, _ => false, out bool cubeMissing);

            Assert.Null(env);
            Assert.False(cubeMissing);
        }

        [Fact]
        public void Resolve_BgsmEnvironment_UsesTheMaterialScaleAndNoMask()
        {
            var bgsm = ParseBgsm(new BgsmFields
            {
                Base = new MaterialBaseFields { EnvironmentMapping = true, EnvironmentMappingMaskScale = 1.25f },
                Envmap = "textures\\shared\\cubemaps\\outsideoldtownreflectcube_e.dds",
            });

            var env = EnvironmentParams.Resolve(bgsm.EnvironmentMapping, bgsm.EnvmapTexture, null, bgsm.EnvironmentMappingMaskScale, _ => true, out _);

            Assert.Equal(new EnvironmentParams("textures\\shared\\cubemaps\\outsideoldtownreflectcube_e.dds", "", 1.25f), env);
        }

        internal static BGEM ParseBgem(BgemFields fields)
        {
            using var file = TempMaterialFile.Write(fields.Build(2));
            return file.Use(stream =>
            {
                var bgem = new BGEM();
                bgem.Open(stream);
                return bgem;
            });
        }

        private static BGSM ParseBgsm(BgsmFields fields)
        {
            using var file = TempMaterialFile.Write(fields.Build(2));
            return file.Use(stream =>
            {
                var bgsm = new BGSM();
                bgsm.Open(stream);
                return bgsm;
            });
        }
    }
}
