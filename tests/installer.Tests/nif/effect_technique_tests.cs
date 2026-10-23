using System.Numerics;
using AssetConverter;
using AssetConverter.ShaderTranslation;
using AssetConverter.ShaderTranslation.Archive;
using AssetConverter.ShaderTranslation.Dxbc;
using InstallerTests.Support;
using Xunit;
using static InstallerTests.Support.Sm5Assembler;
using SignatureElements = (string Name, uint Index, uint SystemValue, uint Register, byte Mask)[];

namespace InstallerTests.Nif
{
    // Fixtures are the outputs of the reference descriptor implementation (the effect technique spec's fit function) for the same inputs.
    public class EffectTechniqueTests
    {
        private const uint VertexFlagsUvNormalColor = 0x2 | 0x8 | 0x20;

        private static EffectShapeFacts Mesh(
            uint flags1 = 0, uint flags2 = 0, int? alphaFlags = null, string source = "a.dds", string greyscale = "", uint vertexFlags = VertexFlagsUvNormalColor,
            EffectMaterialParams? material = null, bool skinInstance = false) => new(
            flags1, flags2, Particle: false, ParticleVertexColors: false, ParticleSubtextures: 0, vertexFlags, skinInstance,
            alphaFlags is { } flags ? EffectAlphaState.FromFlags(flags, 0) : null, source, greyscale, "", "", material);

        private static int Alpha(int source, int destination) => 1 | (source << 1) | (destination << 5);

        private static EffectMaterialParams Bgem(
            BaseMaterialFile.AlphaBlendModeType blend, string baseTexture = "", string greyscale = "", bool paletteColor = false, bool paletteAlpha = false,
            bool falloff = false, bool colorFalloff = false, bool soft = false, bool lit = false, bool env = false) => new(
            baseTexture, greyscale, "", "", "", "", "", "", blend, false, 0, true, false, false, true, true,
            paletteColor, paletteAlpha, falloff, colorFalloff, soft, lit, env, Vector4.One, 1f, 1f, 1f, 0f, 0f, 100f, 1f, 1f, Vector3.One, 0f, 1f,
            Vector2.Zero, Vector2.One);

        private static readonly Dictionary<string, (EffectShapeFacts Facts, uint Expected)> Scenarios = new()
        {
            ["additive blend with palette colour and alpha"] = (Mesh(flags1: 1u << 4 | 1u << 5, flags2: 1u << 5, greyscale: "g.dds", alphaFlags: Alpha(6, 0)), 0x00006025),
            ["multiplicative (zero, source colour)"] = (Mesh(alphaFlags: Alpha(1, 2)), 0x00000044),
            ["multiplicative (destination colour, zero)"] = (Mesh(alphaFlags: Alpha(4, 1)), 0x00000044),
            ["standard blend"] = (Mesh(alphaFlags: Alpha(6, 7)), 0x00000004),
            ["blend switched off with destination factor one"] = (Mesh(alphaFlags: 6 << 1), 0x00000004),
            ["falloff pulls in vertex normals"] = (Mesh(flags1: 1u << 6), 0x00800014),
            ["colour falloff on top of falloff"] = (Mesh(flags1: 1u << 6 | 1u << 8), 0x00a00014),
            ["environment mapping pulls in vertex normals"] = (Mesh(flags1: 1u << 7), 0x00880004),
            ["soft depth, effect lighting and decal"] = (Mesh(flags1: 1u << 30 | 1u << 26, flags2: 1u << 30), 0x10001404),
            ["no source texture means no uv"] = (Mesh(source: ""), 0x00000000),
            ["palette flags without a greyscale texture do nothing"] = (Mesh(flags1: 1u << 4 | 1u << 5), 0x00000004),
            ["material file overrides the shader flags"] = (Mesh(flags1: 1u << 6, material: Bgem(BaseMaterialFile.AlphaBlendModeType.Additive, "b.dds", "g.dds", paletteColor: true, soft: true, lit: true)), 0x00003424),
            ["material file with standard blend, palette alpha, falloff and environment"] = (Mesh(material: Bgem(BaseMaterialFile.AlphaBlendModeType.Standard, paletteAlpha: true, falloff: true, colorFalloff: true, env: true)), 0x00a80010),
        };

        public static TheoryData<string> ScenarioNames() => new(Scenarios.Keys);

        [Theory]
        [MemberData(nameof(ScenarioNames))]
        public void Descriptor_MeshShape_MatchesTheReferenceTechnique(string scenario)
        {
            var (facts, expected) = Scenarios[scenario];
            uint descriptor = EffectTechnique.Descriptor(EffectTechnique.Features(facts));

            Assert.True(expected == descriptor, $"{scenario}: expected {expected:x8}, got {descriptor:x8}");
        }

        [Fact]
        public void Descriptor_ParticleSystem_SetsParticleSubtextureColourAndNormalMaskBitsAndIgnoresFalloff()
        {
            var facts = new EffectShapeFacts(1u << 6 | 1u << 8, 1u << 5, Particle: true, ParticleVertexColors: true, ParticleSubtextures: 4, MeshVertexFlags: 0,
                HasSkinInstance: false, EffectAlphaState.FromFlags(Alpha(6, 0), 0), "a.dds", "", "n.dds", "m.dds", null);

            uint descriptor = EffectTechnique.Descriptor(EffectTechnique.Features(facts));

            Assert.Equal(0x004000adu, descriptor);
        }

        [Theory]
        [InlineData("", "", "n.dds", "m.dds", true)]
        [InlineData("n.dds", "m.dds", "", "", false)]
        [InlineData("n.dds", "", "n.dds", "m.dds", true)]
        public void Features_ParticleNormalMask_ReadsTheMaterialFileTexturesWhenOneResolves(string shaderNormal, string shaderEnvMask, string materialNormal, string materialEnvMask, bool expected)
        {
            var material = Bgem(BaseMaterialFile.AlphaBlendModeType.Standard, "a.dds") with { NormalTexture = materialNormal, EnvMaskTexture = materialEnvMask };
            var facts = new EffectShapeFacts(0, 0, Particle: true, ParticleVertexColors: false, ParticleSubtextures: 0, MeshVertexFlags: 0,
                HasSkinInstance: false, null, "a.dds", "", shaderNormal, shaderEnvMask, material);

            Assert.Equal(expected, EffectTechnique.Features(facts).ParticleNormalMask);
        }

        [Fact]
        public void Features_ParticleNormalMask_WithoutAMaterialFileReadsTheShaderTextures()
        {
            var facts = new EffectShapeFacts(0, 0, Particle: true, ParticleVertexColors: false, ParticleSubtextures: 0, MeshVertexFlags: 0,
                HasSkinInstance: false, null, "a.dds", "", "n.dds", "m.dds", null);

            Assert.True(EffectTechnique.Features(facts).ParticleNormalMask);
        }

        [Fact]
        public void Descriptor_ParticleSystemWithoutSourceTexture_HasNeitherUvNorSubtextures()
        {
            var facts = new EffectShapeFacts(0, 0, Particle: true, ParticleVertexColors: false, ParticleSubtextures: 4, MeshVertexFlags: 0,
                HasSkinInstance: false, null, "", "", "", "", null);

            Assert.Equal(0x80u, EffectTechnique.Descriptor(EffectTechnique.Features(facts)));
        }

        [Fact]
        public void Descriptor_VertexColoursNeedBothTheShaderFlagAndTheColourStream()
        {
            Assert.Equal(0x5u, EffectTechnique.Descriptor(EffectTechnique.Features(Mesh(flags2: 1u << 5))));
            Assert.Equal(0x4u, EffectTechnique.Descriptor(EffectTechnique.Features(Mesh(flags2: 1u << 5, vertexFlags: 0x2 | 0x8))));
            Assert.Equal(0x4u, EffectTechnique.Descriptor(EffectTechnique.Features(Mesh())));
        }

        [Fact]
        public void Features_SkinnedByVertexFlagOrSkinInstance()
        {
            Assert.True(EffectTechnique.Features(Mesh(vertexFlags: VertexFlagsUvNormalColor | 0x40)).Skinned);
            Assert.True(EffectTechnique.Features(Mesh(skinInstance: true)).Skinned);
            Assert.False(EffectTechnique.Features(Mesh()).Skinned);
        }

        [Theory]
        [InlineData(0x0000148du, 0x0000148du)]
        [InlineData(0x0000748du, 0x0000148du)]
        [InlineData(0x4800e0adu, 0x000000adu)]
        public void PairVertexDescriptor_ClearsThePixelOnlyBits(uint pixel, uint expectedVertex)
        {
            var catalog = Catalog((expectedVertex, pixel));

            Assert.Equal(expectedVertex, EffectTechnique.PairVertexDescriptor(pixel, catalog));
        }

        [Fact]
        public void PairVertexDescriptor_RetriesWithoutBlendAndDecalBitsWhenTheFullDescriptorHasNoVertexShader()
        {
            uint pixel = 0x10000021u;
            var catalog = Catalog((0x00000001u, pixel));

            Assert.Equal(0x00000001u, EffectTechnique.PairVertexDescriptor(pixel, catalog));
        }

        [Fact]
        public void PairVertexDescriptor_NoMatchingVertexShader_IsNull()
        {
            Assert.Null(EffectTechnique.PairVertexDescriptor(0x00001400u, Catalog((0x4u, 0x4u))));
        }

        [Fact]
        public void ProvidedAttributes_FollowTheVertexDescriptorBits()
        {
            var attributes = EffectTechnique.ProvidedAttributes(EffectDescriptorBits.Uv | EffectDescriptorBits.Normals | EffectDescriptorBits.EnvMapping | EffectDescriptorBits.VertexColors | EffectDescriptorBits.Particle);

            Assert.Equal(["BINORMAL0", "COLOR0", "NORMAL0", "POSITION0", "TEXCOORD0", "TEXCOORD2"], attributes.Order().ToArray());
            Assert.Equal(["POSITION0"], EffectTechnique.ProvidedAttributes(0).ToArray());
        }

        [Fact]
        public void Select_SupportedShape_NamesBothShadersAfterTheirDescriptors()
        {
            var facts = Mesh(flags1: 1u << 4 | 1u << 5, greyscale: "g.dds");

            var selection = EffectTechnique.Select(facts, Catalog((0x4u, 0x6004u)));

            Assert.Null(selection.HiddenReason);
            Assert.Equal("effect_vs_00000004", selection.VertexName);
            Assert.Equal("effect_ps_00006004", selection.PixelName);
        }

        [Fact]
        public void Select_SkinnedShape_IsHiddenWithoutConsultingTheCatalogue()
        {
            var selection = EffectTechnique.Select(Mesh(skinInstance: true), Catalog());

            Assert.Equal(EffectTechnique.SkinnedReason, selection.HiddenReason);
        }

        [Fact]
        public void Select_DescriptorWithoutPixelShader_IsHidden()
        {
            var selection = EffectTechnique.Select(Mesh(), Catalog((0x1u, 0x1u)));

            Assert.Equal(EffectTechnique.NoPixelShaderReason, selection.HiddenReason);
        }

        [Fact]
        public void Select_PixelShaderWithoutVertexShader_IsHidden()
        {
            var selection = EffectTechnique.Select(Mesh(), Catalog((0x80u, 0x4u)));

            Assert.Equal(EffectTechnique.NoVertexShaderReason, selection.HiddenReason);
        }

        [Fact]
        public void Catalog_PixelInputTheVertexShaderDoesNotWrite_IsASignatureMismatch()
        {
            var catalog = FxpCatalog(
                (Vertex(inputs: [("POSITION", 0u, 0u, 0u, (byte)0xF)], outputs: [("SV_Position", 0u, 1u, 0u, (byte)0xF)]), true),
                (Pixel(inputs: [("SV_Position", 0u, 1u, 0u, (byte)0xF), ("TEXCOORD", 0u, 0u, 1u, (byte)0x3)]), false));

            string? mismatch = catalog.CheckPair(0x4u, 0x4u, EffectTechnique.ProvidedAttributes(0x4u));

            Assert.Contains("TEXCOORD0", mismatch);
        }

        [Fact]
        public void Catalog_PixelReadsMoreComponentsThanTheVertexShaderWrites_IsASignatureMismatch()
        {
            var catalog = FxpCatalog(
                (Vertex(inputs: [("POSITION", 0u, 0u, 0u, (byte)0xF)], outputs: [("TEXCOORD", 0u, 0u, 1u, (byte)0x3)]), true),
                (Pixel(inputs: [("TEXCOORD", 0u, 0u, 1u, (byte)0xF)]), false));

            Assert.Contains("components", catalog.CheckPair(0x4u, 0x4u, EffectTechnique.ProvidedAttributes(0x4u)));
        }

        [Fact]
        public void Catalog_PixelInputAtAnotherRegister_IsASignatureMismatch()
        {
            var catalog = FxpCatalog(
                (Vertex(inputs: [("POSITION", 0u, 0u, 0u, (byte)0xF)], outputs: [("TEXCOORD", 0u, 0u, 2u, (byte)0xF)]), true),
                (Pixel(inputs: [("TEXCOORD", 0u, 0u, 1u, (byte)0xF)]), false));

            Assert.NotNull(catalog.CheckPair(0x4u, 0x4u, EffectTechnique.ProvidedAttributes(0x4u)));
        }

        [Fact]
        public void Catalog_VertexInputTheGeometryDoesNotProvide_IsAMismatch()
        {
            var catalog = FxpCatalog(
                (Vertex(inputs: [("POSITION", 0u, 0u, 0u, (byte)0xF), ("TEXCOORD", 0u, 0u, 1u, (byte)0x3)], outputs: [("TEXCOORD", 0u, 0u, 1u, (byte)0x3)]), true),
                (Pixel(inputs: [("TEXCOORD", 0u, 0u, 1u, (byte)0x3)]), false));

            Assert.Contains("TEXCOORD0", catalog.CheckPair(0x4u, 0x4u, EffectTechnique.ProvidedAttributes(0u)));
        }

        [Fact]
        public void Catalog_MatchingSignatures_Link()
        {
            var catalog = FxpCatalog(
                (Vertex(inputs: [("POSITION", 0u, 0u, 0u, (byte)0xF), ("TEXCOORD", 0u, 0u, 1u, (byte)0x3)], outputs: [("SV_Position", 0u, 1u, 0u, (byte)0xF), ("TEXCOORD", 0u, 0u, 1u, (byte)0x3)]), true),
                (Pixel(inputs: [("SV_Position", 0u, 1u, 0u, (byte)0xF), ("TEXCOORD", 0u, 0u, 1u, (byte)0x3)]), false));

            Assert.Null(catalog.CheckPair(0x4u, 0x4u, EffectTechnique.ProvidedAttributes(0x4u)));
            Assert.True(catalog.HasVertexShader(0x4u));
            Assert.True(catalog.HasPixelShader(0x4u));
            Assert.False(catalog.HasPixelShader(0x8u));
        }

        [Fact]
        public void Catalog_DuplicateDescriptorInOneStage_IsRejected()
        {
            var vertex = Vertex([], []);

            Assert.Throws<InvalidDataException>(() => FxpCatalog((vertex, true), (vertex, true)));
        }

        [Fact]
        public void Catalog_ArchiveWithoutTheEffectGroup_IsRejected()
        {
            var fxp = new FxpFile([new FxpGroupHeader(0, 0, 0, 0, 0)], []);

            Assert.Throws<InvalidDataException>(() => new FxpEffectShaderCatalog(fxp));
        }

        private static byte[] Vertex(SignatureElements inputs, SignatureElements outputs) => Shader(ShaderStage.Vertex, inputs, outputs);

        private static byte[] Pixel(SignatureElements inputs) => Shader(ShaderStage.Pixel, inputs, []);

        private static byte[] Shader(ShaderStage stage, SignatureElements inputs, SignatureElements outputs) =>
            new Sm5Assembler(stage).Temps(1).Instruction(Opcode.Ret, 0)
                .Chunk("ISGN", Signature(inputs))
                .Chunk("OSGN", Signature(outputs))
                .BuildDxbc();

        private static FxpEffectShaderCatalog FxpCatalog(params (byte[] Dxbc, bool IsVertex)[] shaders)
        {
            var vertexShaders = shaders.Where(s => s.IsVertex).ToList();
            var ordered = vertexShaders.Concat(shaders.Where(s => !s.IsVertex)).ToList();
            var groups = Enumerable.Range(0, FxpEffectShaderCatalog.EffectGroup + 1)
                .Select(i => i == FxpEffectShaderCatalog.EffectGroup
                    ? new FxpGroupHeader((uint)vertexShaders.Count, 0, 0, (uint)(ordered.Count - vertexShaders.Count), 0)
                    : new FxpGroupHeader(0, 0, 0, 0, 0)).ToList();
            var entries = ordered.Select((s, i) => new FxpEntry(i, FxpEffectShaderCatalog.EffectGroup, 0x4u, ReadOnlyMemory<byte>.Empty, s.Dxbc)).ToList();
            return new FxpEffectShaderCatalog(new FxpFile(groups, entries));
        }

        private static StubCatalog Catalog(params (uint Vertex, uint Pixel)[] pairs) =>
            new(pairs.Select(p => p.Vertex).ToHashSet(), pairs.Select(p => p.Pixel).ToHashSet());

        private sealed class StubCatalog(IReadOnlySet<uint> vertex, IReadOnlySet<uint> pixel) : IEffectShaderCatalog
        {
            public bool HasVertexShader(uint descriptor) => vertex.Contains(descriptor);

            public bool HasPixelShader(uint descriptor) => pixel.Contains(descriptor);

            public string? CheckPair(uint vertexDescriptor, uint pixelDescriptor, IReadOnlySet<string> providedAttributes) => null;
        }
    }
}
