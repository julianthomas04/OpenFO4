using System.Numerics;
using System.Text.Json.Nodes;
using AssetConverter;
using AssetConverter.ShaderTranslation;
using AssetConverter.ShaderTranslation.Archive;
using AssetConverter.ShaderTranslation.Dxbc;
using InstallerTests.Archives;
using InstallerTests.Support;
using NiflySharp;
using NiflySharp.Blocks;
using NiflySharp.Structs;
using SharpGLTF.Schema2;
using Xunit;
using static InstallerTests.Support.Sm5Assembler;

namespace InstallerTests.Nif
{
    /// <summary>Real effect-group catalogue stand-in: one vertex and one pixel shader per descriptor pair, with signatures that link.</summary>
    internal static class EffectCatalogFixture
    {
        public static FxpEffectShaderCatalog With(params (uint Vertex, uint Pixel)[] pairs)
        {
            var vertex = pairs.Select(p => (p.Vertex, Dxbc: Stage(ShaderStage.Vertex, p.Vertex))).DistinctBy(v => v.Vertex).ToList();
            var pixel = pairs.Select(p => (p.Pixel, Dxbc: Stage(ShaderStage.Pixel, p.Pixel))).DistinctBy(v => v.Pixel).ToList();
            var groups = Enumerable.Range(0, FxpEffectShaderCatalog.EffectGroup + 1)
                .Select(i => i == FxpEffectShaderCatalog.EffectGroup
                    ? new FxpGroupHeader((uint)vertex.Count, 0, 0, (uint)pixel.Count, 0)
                    : new FxpGroupHeader(0, 0, 0, 0, 0)).ToList();
            var entries = vertex.Select(v => (v.Vertex, v.Dxbc)).Concat(pixel.Select(p => (p.Pixel, p.Dxbc)))
                .Select((e, i) => new FxpEntry(i, FxpEffectShaderCatalog.EffectGroup, e.Item1, ReadOnlyMemory<byte>.Empty, e.Dxbc)).ToList();
            return new FxpEffectShaderCatalog(new FxpFile(groups, entries));
        }

        private static byte[] Stage(ShaderStage stage, uint descriptor)
        {
            bool uv = (descriptor & EffectDescriptorBits.Uv) != 0;
            var inputs = new List<(string, uint, uint, uint, byte)>();
            var outputs = new List<(string, uint, uint, uint, byte)>();
            if (stage == ShaderStage.Vertex)
            {
                inputs.Add(("POSITION", 0, 0, 0, 0xF));
                outputs.Add(("SV_Position", 0, 1, 0, 0xF));
                if (uv) inputs.Add(("TEXCOORD", 0, 0, 1, 0x3));
            }
            else
            {
                inputs.Add(("SV_Position", 0, 1, 0, 0xF));
            }
            if (uv)
            {
                if (stage == ShaderStage.Vertex) outputs.Add(("TEXCOORD", 0, 0, 1, 0x3));
                else inputs.Add(("TEXCOORD", 0, 0, 1, 0x3));
            }
            return new Sm5Assembler(stage).Temps(1).Instruction(Opcode.Ret, 0)
                .Chunk("ISGN", Signature([.. inputs]))
                .Chunk("OSGN", Signature([.. outputs]))
                .BuildDxbc();
        }
    }

    // NifConverter.EffectCatalog is process-wide state: the tests of this class always restore it.
    [Collection(StaticStateCollections.NifConverterState)]
    public sealed class EffectConversionTests : IDisposable
    {
        private static readonly Vector3[] Triangle = [new(0, 0, 0), new(10, 0, 0), new(0, 10, 0)];
        private static readonly Triangle[] OneTriangle = [new(0, 1, 2)];
        private static readonly TexCoord[] Uvs = [new(0f, 0f), new(1f, 0f), new(0f, 1f)];

        public EffectConversionTests() => NifConverter.EffectCatalog = EffectCatalogFixture.With((0x4, 0x10000004));

        public void Dispose() => NifConverter.EffectCatalog = null;

        [Fact]
        public void ConvertToGltf_BarrelMistShape_DrawsThroughEffectExtrasWithoutObjectShaderTokens()
        {
            using var dir = new ArchiveTempDir();
            var scene = MistScene();

            var model = Convert(dir, scene);

            var material = model.LogicalMaterials.Single();
            Assert.Matches(@"^effects/misttilelight_d\.dds\|\|blend\|fx\|eff:[0-9a-f]{8}\|ts$", material.Name);
            var effect = material.Extras["effect"]!;
            Assert.Equal("effect_vs_00000004", effect["vs"]!.GetValue<string>());
            Assert.Equal("effect_ps_10000004", effect["ps"]!.GetValue<string>());
            Assert.Equal("alpha", effect["blend"]!["mode"]!.GetValue<string>());
            Assert.Equal("effects/misttilelight_d.dds", effect["textures"]!["source"]!.GetValue<string>());
            Assert.Equal(2348810240u, effect["flags1"]!.GetValue<uint>());
        }

        [Fact]
        public void ConvertToGltf_EffectMaterialNameKey_IsTheHashOfTheExtras()
        {
            using var dir = new ArchiveTempDir();

            var material = Convert(dir, MistScene()).LogicalMaterials.Single();

            uint key = GltfMaterialExtras.KeyOf((JsonObject)JsonNode.Parse(material.Extras["effect"]!.ToJsonString())!);
            Assert.EndsWith($"|eff:{key:x8}|ts", material.Name);
        }

        [Fact]
        public void ConvertToGltf_TwoEffectShapesWithDifferentParameters_GetDifferentMaterialNames()
        {
            using var dir = new ArchiveTempDir();
            var scene = MistScene();
            scene.AttachShader(scene.AddShape(scene.Root, "Other", Triangle, OneTriangle, Uvs),
                EffectShaderFixture.Create(2348810240, 48, "textures\\Effects\\MistTileLight_d.dds", baseColorScale: 3f));

            var model = Convert(dir, scene);

            Assert.Equal(2, model.LogicalMaterials.Select(m => m.Name).Distinct().Count());
        }

        [Fact]
        public void ConvertToGltf_EffectShapeWhoseDescriptorIsNotInTheCatalogue_StaysOnTheObjectShaderPath()
        {
            NifConverter.EffectCatalog = EffectCatalogFixture.With((0x1, 0x1));
            using var dir = new ArchiveTempDir();

            var material = Convert(dir, MistScene()).LogicalMaterials.Single();

            Assert.Equal("effects/misttilelight_d.dds||blend|fx|bc:FFFFFF|bs:1|ts", material.Name);
            Assert.Null(material.Extras);
            Assert.Contains(NifAnimationStats.SummaryLines(), l => l.StartsWith(EffectCounters.HiddenPrefix + EffectTechnique.NoPixelShaderReason, StringComparison.Ordinal));
        }

        [Fact]
        public void ConvertToGltf_EffectShapeWithAProceduralLightningController_StaysOnTheObjectShaderPath()
        {
            using var dir = new ArchiveTempDir();
            var scene = MistScene();
            var shader = (BSEffectShaderProperty)scene.Nif.Blocks.Single(b => b is BSEffectShaderProperty);
            shader.Controller = new NiBlockRef<NiTimeController>(scene.Nif.AddBlock(new BSProceduralLightningController
            {
                NextController = new NiBlockRef<NiTimeController>(-1),
                Flags = new NiflySharp.Bitfields.TimeControllerFlags(),
                Interpolator1_Generation = new NiBlockRef<NiInterpolator>(-1),
                Interpolator2_Mutation = new NiBlockRef<NiInterpolator>(-1),
                Interpolator3_Subdivision = new NiBlockRef<NiInterpolator>(-1),
                Interpolator4_NumBranches = new NiBlockRef<NiInterpolator>(-1),
                Interpolator5_NumBranchesVar = new NiBlockRef<NiInterpolator>(-1),
                Interpolator6_Length = new NiBlockRef<NiInterpolator>(-1),
                Interpolator7_LengthVar = new NiBlockRef<NiInterpolator>(-1),
                Interpolator8_Width = new NiBlockRef<NiInterpolator>(-1),
                Interpolator9_ArcOffset = new NiBlockRef<NiInterpolator>(-1),
            }));

            var material = Convert(dir, scene).LogicalMaterials.Single();

            Assert.Null(material.Extras);
            Assert.Contains(NifAnimationStats.SummaryLines(), l => l.StartsWith(EffectCounters.HiddenPrefix + EffectCounters.ProceduralLightningReason, StringComparison.Ordinal));
        }

        [Fact]
        public void ConvertToGltf_EffectShapeNamedByABoltGeneratorNode_StaysOnTheObjectShaderPath()
        {
            using var dir = new ArchiveTempDir();
            var scene = MistScene();
            int shader = scene.Nif.Blocks.IndexOf(scene.Nif.Blocks.Single(b => b is BSEffectShaderProperty));
            var generator = scene.AddNode(scene.Root, "Bolt", System.Numerics.Vector3.Zero);
            generator.Children = new NiBlockRefArray<NiAVObject>();
            generator.Controller = new NiBlockRef<NiTimeController>(scene.Nif.AddBlock(new BSProceduralLightningController
            {
                NextController = new NiBlockRef<NiTimeController>(-1),
                Flags = new NiflySharp.Bitfields.TimeControllerFlags(),
                ShaderProperty = new NiBlockRef<BSShaderProperty>(shader),
                Interpolator1_Generation = new NiBlockRef<NiInterpolator>(-1),
                Interpolator2_Mutation = new NiBlockRef<NiInterpolator>(-1),
                Interpolator3_Subdivision = new NiBlockRef<NiInterpolator>(-1),
                Interpolator4_NumBranches = new NiBlockRef<NiInterpolator>(-1),
                Interpolator5_NumBranchesVar = new NiBlockRef<NiInterpolator>(-1),
                Interpolator6_Length = new NiBlockRef<NiInterpolator>(-1),
                Interpolator7_LengthVar = new NiBlockRef<NiInterpolator>(-1),
                Interpolator8_Width = new NiBlockRef<NiInterpolator>(-1),
                Interpolator9_ArcOffset = new NiBlockRef<NiInterpolator>(-1),
            }));

            var material = Convert(dir, scene).LogicalMaterials.Single();

            Assert.Null(material.Extras);
            Assert.Contains(NifAnimationStats.SummaryLines(), l => l.StartsWith(EffectCounters.HiddenPrefix + EffectCounters.ProceduralLightningReason, StringComparison.Ordinal));
        }

        [Fact]
        public void ConvertToGltf_EffectShapeWhoseControllerChainIsBroken_StaysHiddenInsteadOfFailingTheFile()
        {
            using var dir = new ArchiveTempDir();
            var scene = MistScene();
            var shader = (BSEffectShaderProperty)scene.Nif.Blocks.Single(b => b is BSEffectShaderProperty);
            shader.Controller = new NiBlockRef<NiTimeController>(scene.Nif.Blocks.IndexOf(shader));

            var material = Convert(dir, scene).LogicalMaterials.Single();

            Assert.Null(material.Extras);
            Assert.Contains(NifAnimationStats.SummaryLines(), l => l.StartsWith(EffectCounters.HiddenPrefix + EffectCounters.UnreadableReason, StringComparison.Ordinal));
        }

        [Fact]
        public void ConvertToGltf_RefractionShapeWhoseControllerChainIsBroken_StaysAPlainLightingShape()
        {
            using var dir = new ArchiveTempDir();
            var scene = RefractionScene(flags1: 0x80008001u);
            var shader = (BSLightingShaderProperty)scene.Nif.Blocks.Single(b => b is BSLightingShaderProperty);
            shader.Controller = new NiBlockRef<NiTimeController>(scene.Nif.Blocks.IndexOf(shader));

            var material = Convert(dir, scene).LogicalMaterials.Single();

            Assert.Null(material.Extras);
            Assert.Contains(NifAnimationStats.SummaryLines(), l => l.StartsWith(EffectCounters.RefractionSkippedPrefix + EffectCounters.UnreadableReason, StringComparison.Ordinal));
        }

        [Fact]
        public void ConvertToGltf_WithoutACatalogue_EffectShapesStayOnTheObjectShaderPathAndAreCounted()
        {
            NifConverter.EffectCatalog = null;
            using var dir = new ArchiveTempDir();

            var material = Convert(dir, MistScene()).LogicalMaterials.Single();

            Assert.DoesNotContain("eff:", material.Name);
            Assert.Contains(NifAnimationStats.SummaryLines(), l => l.StartsWith(EffectCounters.HiddenPrefix + EffectCounters.NoCatalogReason, StringComparison.Ordinal));
        }

        [Fact]
        public void ConvertToGltf_OneShaderRead_FeedsTheEffectExtrasAndTheObjectShaderTokensAlike()
        {
            using var dir = new ArchiveTempDir();
            var withExtras = Convert(dir, MistScene()).LogicalMaterials.Single();
            NifConverter.EffectCatalog = null;
            var leftover = Convert(dir, MistScene()).LogicalMaterials.Single();

            var extras = withExtras.Extras["effect"]!;
            var color = extras["base_color"]!.AsArray().Select(channel => channel!.GetValue<float>()).ToArray();
            string expectedColor = $"{(int)MathF.Round(color[0] * 255f):X2}{(int)MathF.Round(color[1] * 255f):X2}{(int)MathF.Round(color[2] * 255f):X2}";
            string[] tokens = leftover.Name.Split('|');
            Assert.DoesNotContain(withExtras.Name.Split('|'), segment => segment.StartsWith("bc:", StringComparison.Ordinal) || segment.StartsWith("bs:", StringComparison.Ordinal));
            Assert.Contains($"bc:{expectedColor}", tokens);
            Assert.Contains($"bs:{extras["base_color_scale"]!.GetValue<float>().ToString("0.####", System.Globalization.CultureInfo.InvariantCulture)}", tokens);
            Assert.Null(leftover.Extras);
        }
        [Fact]
        public void ConvertToGltf_RefractionShape_GetsTheRefrTokenAndRefractionExtrasWithBothNormalMaps()
        {
            using var dir = new ArchiveTempDir();
            var scene = RefractionScene(flags1: 0x80008001u);

            var material = Convert(dir, scene).LogicalMaterials.Single();

            Assert.Matches(@"^effects/smokevapor02tile_n\.dds\|effects/smokevapor01tile_n\.dds\|refr\|eff:[0-9a-f]{8}", material.Name);
            Assert.Null(material.Extras["effect"]);
            var refraction = material.Extras["refraction"]!;
            Assert.Equal(0.04f, refraction["strength"]!.GetValue<float>(), 1e-6f);
            Assert.Equal("effects/smokevapor02tile_n.dds", refraction["normal_a"]!.GetValue<string>());
            Assert.Equal("effects/smokevapor01tile_n.dds", refraction["normal_b"]!.GetValue<string>());
            Assert.False(refraction["vertex_colors"]!.GetValue<bool>());
        }

        [Fact]
        public void ConvertToGltf_LightingShapeWithoutTheRefractionFlag_IsAnOrdinaryMaterial()
        {
            using var dir = new ArchiveTempDir();
            var scene = RefractionScene(flags1: 0x80000001u);

            var material = Convert(dir, scene).LogicalMaterials.Single();

            Assert.DoesNotContain("refr", material.Name);
            Assert.Null(material.Extras);
        }

        // A freshly created shape carries the skinned vertex attribute (top bits of the vertex description) and an unset skin reference of 0.
        private static void Unskin(BSTriShape shape)
        {
            shape.SkinInstanceRef = new NiBlockRef<NiObject>(-1);
            shape.VertexDesc.Value &= ~(0x40UL << 44);
        }

        private static NifSceneBuilder MistScene()
        {
            var scene = new NifSceneBuilder();
            var shape = scene.AddShape(scene.Root, "Mist", Triangle, OneTriangle, Uvs);
            Unskin(shape);
            scene.AttachShader(shape, EffectShaderFixture.Create(
                2348810240, 48, "textures\\Effects\\MistTileLight_d.dds", "textures\\Effects\\Gradients\\DustAlphaBasicGrad.dds"));
            shape.AlphaPropertyRef = new NiBlockRef<NiAlphaProperty>(scene.Nif.AddBlock(
                new NiAlphaProperty { Flags = new NiflySharp.Bitfields.AlphaFlags { Value = 4333 }, Threshold = 128 }));
            return scene;
        }

        private static NifSceneBuilder RefractionScene(uint flags1)
        {
            var scene = new NifSceneBuilder();
            var shape = scene.AddShape(scene.Root, "ref003:0", Triangle, OneTriangle, Uvs);
            Unskin(shape);
            int textures = scene.Nif.AddBlock(new BSShaderTextureSet
            {
                Textures = new[] { "textures\\Effects\\SmokeVapor02Tile_n.dds", "textures\\Effects\\SmokeVapor01Tile_n.dds", "", "", "", "", "", "", "", "" }
                    .Select(path => new NiString4(path, false)).ToList(),
            });
            scene.AttachShader(shape, EffectShaderFixture.CreateRefraction(flags1, 33, 0.04f, textures));
            return scene;
        }

        private static ModelRoot Convert(ArchiveTempDir dir, NifSceneBuilder scene)
        {
            string source = scene.Save(dir.Path, "model.nif");
            string destination = Path.Combine(dir.Path, "model.glb");
            NifConverter.ConvertToGltf(source, destination);
            return ModelRoot.Load(destination);
        }
    }
}
