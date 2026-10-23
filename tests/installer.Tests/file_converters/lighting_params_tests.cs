using System.Reflection;
using AssetConverter;
using NiflySharp;
using NiflySharp.Blocks;
using NiflySharp.Enums;
using NiflySharp.Structs;
using Xunit;

namespace InstallerTests
{
    public class LightingParamsTests
    {
        [Fact]
        public void FromBgsm_SpecularAndEmissive_CopiesScalarsAndDropsAlphaByteOfColours()
        {
            var bgsm = new BGSM
            {
                SpecularEnabled = true,
                SpecularColor = 0xFF102030,
                SpecularMult = 2f,
                Smoothness = 0.5f,
                FresnelPower = 4f,
                EmitEnabled = true,
                EmittanceColor = 0xFFAABBCC,
                EmittanceMult = 3f,
                SmoothSpecTexture = "textures/spec.dds",
            };

            var lighting = LightingParams.FromBgsm(bgsm);

            Assert.True(lighting.SpecularEnabled);
            Assert.Equal(0x102030, lighting.SpecularColor);
            Assert.Equal(2f, lighting.SpecularMult);
            Assert.Equal(0.5f, lighting.Smoothness);
            Assert.Equal(4f, lighting.FresnelPower);
            Assert.True(lighting.EmissiveEnabled);
            Assert.Equal(0xAABBCC, lighting.EmissiveColor);
            Assert.Equal(3f, lighting.EmissiveMult);
            Assert.Equal("textures/spec.dds", lighting.SpecularTexture);
        }

        [Fact]
        public void FromBgsm_GlowTextureWithoutGlowmapFlag_IsIgnored()
        {
            var bgsm = new BGSM { GlowTexture = "textures/glow.dds", Glowmap = false };

            Assert.Equal("", LightingParams.FromBgsm(bgsm).GlowTexture);
        }

        [Fact]
        public void FromBgsm_GlowTextureWithGlowmapFlag_IsKept()
        {
            var bgsm = new BGSM { GlowTexture = "textures/glow.dds", Glowmap = true };

            Assert.Equal("textures/glow.dds", LightingParams.FromBgsm(bgsm).GlowTexture);
        }

        [Fact]
        public void FromBgsm_NullTexturePaths_BecomeEmptyStrings()
        {
            var bgsm = new BGSM { SmoothSpecTexture = null, GlowTexture = null, Glowmap = true };

            var lighting = LightingParams.FromBgsm(bgsm);

            Assert.Equal("", lighting.SpecularTexture);
            Assert.Equal("", lighting.GlowTexture);
        }

        [Fact]
        public void FromInline_ShaderColoursInUnitRange_PackAsRoundedRgbBytes()
        {
            var shader = Fo4Shader(
                Fallout4ShaderPropertyFlags1.Specular | Fallout4ShaderPropertyFlags1.Own_Emit,
                emissiveColor: new Color4(1f, 0.5f, 0f, 1f));
            SetPrivate(shader, "_specularColor", new Color3(0f, 1f, 0.2f));
            SetPrivate(shader, "_specularStrength", 1.25f);
            SetPrivate(shader, "_smoothness", 0.75f);
            SetPrivate(shader, "_fresnelPower", 6f);
            SetPrivate(shader, "_emissiveMultiple", 2f);

            var lighting = LightingParams.FromInline(shader, null);

            Assert.True(lighting.SpecularEnabled);
            Assert.Equal(0x00FF33, lighting.SpecularColor);
            Assert.Equal(1.25f, lighting.SpecularMult);
            Assert.Equal(0.75f, lighting.Smoothness);
            Assert.Equal(6f, lighting.FresnelPower);
            Assert.True(lighting.EmissiveEnabled);
            Assert.Equal(0xFF8000, lighting.EmissiveColor);
            Assert.Equal(2f, lighting.EmissiveMult);
        }

        [Fact]
        public void FromInline_ColourOutsideUnitRange_ClampsInsteadOfWrapping()
        {
            var shader = Fo4Shader(emissiveColor: new Color4(5f, -3f, 1f, 1f));

            var lighting = LightingParams.FromInline(shader, null);

            Assert.Equal(0xFF00FF, lighting.EmissiveColor);
        }

        [Fact]
        public void FromInline_TextureSlots_SpecularIsSlotSevenAndGlowIsSlotTwoOnlyWithGlowmap()
        {
            var textureSet = TextureSetWithSlots(8);
            textureSet.Textures[2].Content = "glow.dds";
            textureSet.Textures[7].Content = "spec.dds";

            var withGlowmap = LightingParams.FromInline(Fo4Shader(flags2: Fallout4ShaderPropertyFlags2.Glow_Map), textureSet);
            var withoutGlowmap = LightingParams.FromInline(Fo4Shader(), textureSet);

            Assert.Equal("spec.dds", withGlowmap.SpecularTexture);
            Assert.Equal("glow.dds", withGlowmap.GlowTexture);
            Assert.Equal("", withoutGlowmap.GlowTexture);
        }

        [Fact]
        public void FromInline_TextureSetShorterThanSlot_YieldsEmptyPathsInsteadOfThrowing()
        {
            var shortSet = TextureSetWithSlots(3);

            var lighting = LightingParams.FromInline(Fo4Shader(flags2: Fallout4ShaderPropertyFlags2.Glow_Map), shortSet);

            Assert.Equal("", lighting.SpecularTexture);
            Assert.Equal("", lighting.GlowTexture);
        }

        [Fact]
        public void FromInline_NoTextureSet_YieldsEmptyPaths()
        {
            var lighting = LightingParams.FromInline(Fo4Shader(flags2: Fallout4ShaderPropertyFlags2.Glow_Map), null);

            Assert.Equal("", lighting.SpecularTexture);
            Assert.Equal("", lighting.GlowTexture);
        }

        [Fact]
        public void WithTexturePaths_AppliesTheCleanerToBothPathsAndLeavesScalarsAlone()
        {
            var lighting = LightingParams.None with { SpecularTexture = "A", GlowTexture = "B", Smoothness = 0.3f };

            var cleaned = lighting.WithTexturePaths(path => path + "!");

            Assert.Equal("A!", cleaned.SpecularTexture);
            Assert.Equal("B!", cleaned.GlowTexture);
            Assert.Equal(0.3f, cleaned.Smoothness);
        }

        // Shader flag accessors (HasSpecular, Emissive, ...) only resolve on a shader that was read from a
        // Fallout 4 file, so the shader makes a save/load round trip through a real in-memory NIF.
        private static BSLightingShaderProperty Fo4Shader(
            Fallout4ShaderPropertyFlags1 flags1 = 0, Fallout4ShaderPropertyFlags2 flags2 = 0, Color4? emissiveColor = null)
        {
            var authored = new NifFile(NiVersion.GetFO4(), true);
            var shader = new BSLightingShaderProperty
            {
                ShaderFlags_F4SPF1 = flags1,
                ShaderFlags_F4SPF2 = flags2,
                EmissiveColor = emissiveColor ?? new Color4(0f, 0f, 0f, 1f),
            };
            int shaderIndex = authored.AddBlock(shader);
            var shape = new BSTriShape { ShaderPropertyRef = new NiBlockRef<BSShaderProperty>(shaderIndex) };
            int shapeIndex = authored.AddBlock(shape);
            var root = authored.GetRootNode();
            root.Children = new NiBlockRefArray<NiAVObject>();
            root.Children.AddBlockRef(shapeIndex);

            using var stream = new MemoryStream();
            Assert.Equal(0, authored.Save(stream, new NifFileSaveOptions()));
            stream.Position = 0;
            var loaded = new NifFile();
            Assert.Equal(0, loaded.Load(stream, new NifFileLoadOptions()));
            return loaded.Blocks.OfType<BSLightingShaderProperty>().Single();
        }

        private static BSShaderTextureSet TextureSetWithSlots(int count)
        {
            var textureSet = new BSShaderTextureSet();
            textureSet.Textures = new List<NiString4>();
            for (int i = 0; i < count; i++)
                textureSet.Textures.Add(new NiString4 { Content = "" });
            return textureSet;
        }

        private static void SetPrivate(object target, string field, object value) =>
            target.GetType().GetField(field, BindingFlags.NonPublic | BindingFlags.Instance)!.SetValue(target, value);
    }
}
