using System.Numerics;
using AssetConverter;
using InstallerTests.Archives;
using NiflySharp.Enums;
using NiflySharp.Structs;
using SharpGLTF.Schema2;
using Xunit;

namespace InstallerTests.Nif
{
    // Position (x, y, z) -> (x, z, -y) * 0.1, normals only permuted.
    [Collection(StaticStateCollections.NifConverterState)]
    public class NifConverterTests
    {
        private const float Tolerance = 1e-5f;

        private static readonly Vector3[] Triangle = [new(0, 0, 0), new(10, 0, 0), new(0, 10, 0)];
        private static readonly Triangle[] OneTriangle = [new(0, 1, 2)];
        private static readonly TexCoord[] Uvs = [new(0f, 0f), new(1f, 0f), new(0f, 1f)];

        [Fact]
        public void ConvertToGltf_ShapeUnderRoot_ConvertsNodeTransformPositionsAndWinding()
        {
            using var dir = new ArchiveTempDir();
            var scene = new NifSceneBuilder();
            scene.AddShape(scene.Root, "Cube", Triangle, OneTriangle, Uvs, translation: new Vector3(10, 20, 30));

            var model = Convert(dir, scene);

            var node = model.LogicalNodes.Single(n => n.Name == "Cube");
            AssertVector(new Vector3(1f, 3f, -2f), node.LocalTransform.Translation);
            var primitive = node.Mesh.Primitives.Single();
            Assert.Equal([new Vector3(0, 0, 0), new Vector3(1, 0, 0), new Vector3(0, 0, -1)], Positions(primitive), new VectorComparer());
            Assert.Equal([0, 1, 2], primitive.GetIndices());
        }

        [Fact]
        public void ConvertToGltf_Normals_AreOnlyAxisPermutedNotScaled()
        {
            using var dir = new ArchiveTempDir();
            var scene = new NifSceneBuilder();
            Vector3[] normals = [Vector3.UnitZ, Vector3.UnitZ, Vector3.UnitZ];
            scene.AddShape(scene.Root, "Cube", Triangle, OneTriangle, Uvs, normals);

            var model = Convert(dir, scene);

            var normal = model.LogicalMeshes.Single().Primitives.Single().GetVertexAccessor("NORMAL").AsVector3Array()[0];
            Assert.Equal(0f, normal.X, 0.01f);
            Assert.Equal(1f, normal.Y, 0.01f);
            Assert.Equal(0f, normal.Z, 0.01f);
        }

        [Fact]
        public void ConvertToGltf_VertexColours_AreCarriedIntoColor0()
        {
            using var dir = new ArchiveTempDir();
            var scene = new NifSceneBuilder();
            var shape = scene.AddShape(scene.Root, "Blade", Triangle, OneTriangle, Uvs);
            shape.SetVertexColors([new Color4(1f, 0f, 0f, 0f), new Color4(0f, 1f, 0f, 0.5f), new Color4(0f, 0f, 1f, 1f)]);

            var model = Convert(dir, scene);

            var colours = model.LogicalMeshes.Single().Primitives.Single().GetVertexAccessor("COLOR_0").AsVector4Array();
            Assert.Equal(new Vector4(1f, 0f, 0f, 0f), colours[0]);
            Assert.Equal(0.5f, colours[1].W, 1f / 255f);
            Assert.Equal(new Vector4(0f, 0f, 1f, 1f), colours[2]);
        }

        [Fact]
        public void ConvertToGltf_ShapeWithoutVertexColours_GetsWhiteWithZeroSwayWeight()
        {
            using var dir = new ArchiveTempDir();
            var scene = new NifSceneBuilder();
            scene.AddShape(scene.Root, "Cube", Triangle, OneTriangle, Uvs);

            var model = Convert(dir, scene);

            var colours = model.LogicalMeshes.Single().Primitives.Single().GetVertexAccessor("COLOR_0").AsVector4Array();
            Assert.All(colours, colour => Assert.Equal(new Vector4(1f, 1f, 1f, 0f), colour));
        }

        [Fact]
        public void ConvertToGltf_TexCoords_KeepTheNifVUnflipped()
        {
            using var dir = new ArchiveTempDir();
            var scene = new NifSceneBuilder();
            scene.AddShape(scene.Root, "Cube", Triangle, OneTriangle, Uvs);

            var model = Convert(dir, scene);

            var uv = model.LogicalMeshes.Single().Primitives.Single().GetVertexAccessor("TEXCOORD_0").AsVector2Array();
            Assert.Equal(0f, uv[2].X, 1e-3f);
            Assert.Equal(1f, uv[2].Y, 1e-3f);
            Assert.Equal(1f, uv[1].X, 1e-3f);
        }

        [Fact]
        public void ConvertToGltf_EveryVertex_GetsAnOrthonormalTangentWithUnitHandedness()
        {
            using var dir = new ArchiveTempDir();
            var scene = new NifSceneBuilder();
            scene.AddShape(scene.Root, "Cube", Triangle, OneTriangle, Uvs);

            var model = Convert(dir, scene);

            var primitive = model.LogicalMeshes.Single().Primitives.Single();
            var tangents = primitive.GetVertexAccessor("TANGENT").AsVector4Array();
            var normals = primitive.GetVertexAccessor("NORMAL").AsVector3Array();
            Assert.Equal(3, tangents.Count);
            Assert.All(Enumerable.Range(0, 3), i =>
            {
                var t = new Vector3(tangents[i].X, tangents[i].Y, tangents[i].Z);
                Assert.Equal(1f, t.Length(), 1e-3f);
                Assert.Equal(0f, Vector3.Dot(t, normals[i]), 1e-3f);
                Assert.True(tangents[i].W is 1f or -1f);
            });
        }

        [Fact]
        public void ConvertToGltf_RootNodeTransform_IsResetToIdentityButChildrenKeepTheirs()
        {
            using var dir = new ArchiveTempDir();
            var scene = new NifSceneBuilder();
            scene.Root.Translation = new Vector3(100, 200, 300);
            scene.Root.Rotation = NifSceneBuilder.HalfTurnAboutY();
            var child = scene.AddNode(scene.Root, "Door", new Vector3(10, 0, 0));
            scene.AddShape(child, "Leaf", Triangle, OneTriangle, Uvs);

            var model = Convert(dir, scene);

            Assert.Equal(Matrix4x4.Identity, model.LogicalNodes.Single(n => n.Name == "Scene").LocalMatrix);
            AssertVector(new Vector3(1f, 0f, 0f), model.LogicalNodes.Single(n => n.Name == "Door").LocalTransform.Translation);
        }

        [Fact]
        public void ConvertToGltf_NifRotation_MapsToTheSameWorldDirectionAfterAxisConversion()
        {
            // +90 degrees about NIF Z (v' = M v) takes the vertex (10, 0, 0) to (0, 10, 0); in glTF that is (0, 0, -1).
            using var dir = new ArchiveTempDir();
            var scene = new NifSceneBuilder();
            scene.AddShape(scene.Root, "Cube", Triangle, OneTriangle, Uvs, rotation: NifSceneBuilder.AboutZ(90f));

            var model = Convert(dir, scene);

            var node = model.LogicalNodes.Single(n => n.Name == "Cube");
            var local = Positions(node.Mesh.Primitives.Single())[1];
            AssertVector(new Vector3(0f, 0f, -1f), Vector3.Transform(local, node.LocalMatrix));
        }

        [Fact]
        public void ConvertToGltf_NodeScaleAndTranslation_ComposeDownTheHierarchy()
        {
            using var dir = new ArchiveTempDir();
            var scene = new NifSceneBuilder();
            var group = scene.AddNode(scene.Root, "Group", new Vector3(0, 0, 10), scale: 2f);
            scene.AddShape(group, "Leaf", Triangle, OneTriangle, Uvs, translation: new Vector3(5, 0, 0));

            var model = Convert(dir, scene);

            var leaf = model.LogicalNodes.Single(n => n.Name == "Leaf");
            var world = Vector3.Transform(Positions(leaf.Mesh.Primitives.Single())[1], leaf.WorldMatrix);
            // Vertex (10,0,0) + shape offset (5,0,0) = (15,0,0), scaled by 2 = (30,0,0), plus group offset (0,0,10): NIF (30,0,10) -> glTF (3,1,0).
            AssertVector(new Vector3(3f, 1f, 0f), world);
        }

        [Fact]
        public void ConvertToGltf_EditorMarkerShape_IsDropped()
        {
            using var dir = new ArchiveTempDir();
            var scene = new NifSceneBuilder();
            scene.AddShape(scene.Root, "Visible", Triangle, OneTriangle, Uvs);
            scene.AddShape(scene.Root, "VisibilityEditorMarker:0", Triangle, OneTriangle, Uvs);

            var model = Convert(dir, scene);

            Assert.Equal(["Visible"], model.LogicalNodes.Where(n => n.Mesh != null).Select(n => n.Name));
        }

        [Fact]
        public void ConvertToGltf_ShapeWithoutTriangles_IsDropped()
        {
            using var dir = new ArchiveTempDir();
            var scene = new NifSceneBuilder();
            scene.AddShape(scene.Root, "Real", Triangle, OneTriangle, Uvs);
            scene.AddShape(scene.Root, "Empty", Triangle, [], Uvs);

            var model = Convert(dir, scene);

            Assert.Equal(["Real"], model.LogicalNodes.Where(n => n.Mesh != null).Select(n => n.Name));
        }

        [Fact]
        public void ConvertToGltf_UnnamedNode_IsNamedAfterItsBlockIndex()
        {
            using var dir = new ArchiveTempDir();
            var scene = new NifSceneBuilder();
            scene.Root.Name = new NiflySharp.NiStringRef("");
            scene.AddShape(scene.Root, "Leaf", Triangle, OneTriangle, Uvs);

            var model = Convert(dir, scene);

            Assert.Contains(model.LogicalNodes, n => n.Name == "Node_0");
        }

        [Fact]
        public void ConvertToGltf_DegenerateNormal_FallsBackToUpInsteadOfNaN()
        {
            using var dir = new ArchiveTempDir();
            var scene = new NifSceneBuilder();
            Vector3[] zeroNormals = [Vector3.Zero, Vector3.Zero, Vector3.Zero];
            scene.AddShape(scene.Root, "Cube", Triangle, OneTriangle, Uvs, zeroNormals);

            var model = Convert(dir, scene);

            var normals = model.LogicalMeshes.Single().Primitives.Single().GetVertexAccessor("NORMAL").AsVector3Array();
            Assert.All(normals, n => Assert.Equal(1f, n.Length(), 1e-3f));
        }

        [Fact]
        public void ConvertToGltf_ZeroLengthFile_WritesNothing()
        {
            using var dir = new ArchiveTempDir();
            string source = dir.Write("empty.nif", []);
            string destination = Path.Combine(dir.Path, "empty.glb");

            NifConverter.ConvertToGltf(source, destination);

            Assert.False(File.Exists(destination));
        }

        [Fact]
        public void ConvertToGltf_GarbageFile_WritesNothingAndDoesNotThrow()
        {
            using var dir = new ArchiveTempDir();
            string source = dir.Write("junk.nif", TestBytes.Random(2000, 99));
            string destination = Path.Combine(dir.Path, "junk.glb");

            NifConverter.ConvertToGltf(source, destination);

            Assert.False(File.Exists(destination));
        }

        [Fact]
        public void ConvertToGltf_DestinationInMissingFolder_CreatesTheFolder()
        {
            using var dir = new ArchiveTempDir();
            var scene = new NifSceneBuilder();
            scene.AddShape(scene.Root, "Cube", Triangle, OneTriangle, Uvs);
            string source = scene.Save(dir.Path, "cube.nif");
            string destination = Path.Combine(dir.Path, "deep", "er", "cube.glb");

            NifConverter.ConvertToGltf(source, destination);

            Assert.True(File.Exists(destination));
        }

        [Fact]
        public void ConvertToGltf_WaterShaderShape_CarriesTheShaderFieldsInAWaterTokenAndStaysOnTheFxPath()
        {
            using var dir = new ArchiveTempDir();
            var scene = new NifSceneBuilder();
            var shape = scene.AddShape(scene.Root, "Water", Triangle, OneTriangle, Uvs);
            scene.AttachShader(shape, NifSceneBuilder.CreateWaterShader(waterFlags: 452, shaderFlags1: 0x80000008, shaderFlags2: 1, uvOffset: new TexCoord(0.5f, -0.25f), uvScale: new TexCoord(2f, 3f)));

            var model = Convert(dir, scene);

            string name = model.LogicalMaterials.Single().Name;
            Assert.Contains("|fx|", name);
            Assert.EndsWith("|water:452:80000008:00000001:0.5,-0.25:2,3", name);
        }

        [Fact]
        public void ConvertToGltf_InlineAdditiveEffectShader_CarriesEffectTokensAndAdditiveBlendMode()
        {
            using var dir = new ArchiveTempDir();
            var scene = new NifSceneBuilder();
            var shape = scene.AddShape(scene.Root, "Glow", Triangle, OneTriangle, Uvs);
            scene.AttachShader(shape, NifSceneBuilder.CreateEffectShader(
                Fallout4ShaderPropertyFlags1.Use_Falloff | Fallout4ShaderPropertyFlags1.RGB_Falloff, Fallout4ShaderPropertyFlags2.Effect_Lighting,
                sourceTexture: "textures\\Effects\\Glow_d.dds", baseColorScale: 0.5f, baseColor: new Color4(0f, 1f, 0f, 0.5f),
                lightingInfluence: 51, falloff: (1f, -4.371139E-08f, 0.25f, 0f)));
            scene.AttachAlphaProperty(shape, blend: true, AlphaFunction.SRC_ALPHA, AlphaFunction.ONE);

            var model = Convert(dir, scene);

            Assert.Equal("effects/glow_d.dds||blend|0.5|fx|bm:add|bc:00FF00|bs:0.5|fo:1,0,0.25,0,1|li:0.2", model.LogicalMaterials.Single().Name);
        }

        [Theory]
        [InlineData(AlphaFunction.DEST_COLOR, AlphaFunction.ZERO, "bm:mul")]
        [InlineData(AlphaFunction.ZERO, AlphaFunction.SRC_COLOR, "bm:mul")]
        [InlineData(AlphaFunction.SRC_ALPHA, AlphaFunction.INV_SRC_ALPHA, null)]
        [InlineData(AlphaFunction.ONE, AlphaFunction.ONE, "bm:add")]
        public void ConvertToGltf_InlineEffectShaderBlendFactors_SelectTheBlendModeToken(AlphaFunction source, AlphaFunction destination, string? expectedToken)
        {
            using var dir = new ArchiveTempDir();
            var scene = new NifSceneBuilder();
            var shape = scene.AddShape(scene.Root, "Fx", Triangle, OneTriangle, Uvs);
            scene.AttachShader(shape, NifSceneBuilder.CreateEffectShader(0, 0));
            scene.AttachAlphaProperty(shape, blend: true, source, destination);

            var model = Convert(dir, scene);

            string[] segments = model.LogicalMaterials.Single().Name.Split('|');
            Assert.Contains("blend", segments);
            Assert.Equal(expectedToken, segments.SingleOrDefault(segment => segment.StartsWith("bm:", StringComparison.Ordinal)));
        }

        [Fact]
        public void ConvertToGltf_InlineEffectShaderWithoutFalloffOrEffectLighting_OmitsThoseTokens()
        {
            using var dir = new ArchiveTempDir();
            var scene = new NifSceneBuilder();
            var shape = scene.AddShape(scene.Root, "Fx", Triangle, OneTriangle, Uvs);
            scene.AttachShader(shape, NifSceneBuilder.CreateEffectShader(0, 0));

            var model = Convert(dir, scene);

            Assert.Equal("||fx|bc:FFFFFF|bs:1", model.LogicalMaterials.Single().Name);
        }

        [Fact]
        public void ConvertToGltf_TexturelessDoubleSidedInlineEffectShader_IsTwoSided()
        {
            using var dir = new ArchiveTempDir();
            var scene = new NifSceneBuilder();
            var shape = scene.AddShape(scene.Root, "Volume", Triangle, OneTriangle, Uvs);
            scene.AttachShader(shape, NifSceneBuilder.CreateEffectShader(0, Fallout4ShaderPropertyFlags2.Double_Sided));

            var model = Convert(dir, scene);

            Assert.Contains("ts", model.LogicalMaterials.Single().Name.Split('|'));
        }

        [Fact]
        public void ConvertToGltf_InlineEnvironmentMapping_WritesCubeMaskAndScaleTokens()
        {
            using var dir = new ArchiveTempDir();
            var scene = new NifSceneBuilder();
            var shape = scene.AddShape(scene.Root, "Glass", Triangle, OneTriangle, Uvs);
            scene.AttachShader(shape, NifSceneBuilder.CreateEffectShader(
                Fallout4ShaderPropertyFlags1.Environment_Mapping, 0,
                envMapTexture: "textures\\Shared\\Cube_E.dds", envMaskTexture: "textures\\Shared\\Mask_M.dds", envMapScale: 3.5f));

            var model = ConvertWithTextureIndex(dir, scene, "textures\\shared\\cube_e.dds", "textures\\shared\\mask_m.dds");

            Assert.EndsWith("|env:shared/cube_e.dds|envk:shared/mask_m.dds|envs:3.5", model.LogicalMaterials.Single().Name);
        }

        [Fact]
        public void ConvertToGltf_InlineEnvironmentCubeNeedingPathRepair_IsCheckedAfterRepair()
        {
            using var dir = new ArchiveTempDir();
            var scene = new NifSceneBuilder();
            var shape = scene.AddShape(scene.Root, "Glass", Triangle, OneTriangle, Uvs);
            scene.AttachShader(shape, NifSceneBuilder.CreateEffectShader(
                Fallout4ShaderPropertyFlags1.Environment_Mapping, 0, envMapTexture: "textures\\shared\\cube_e.tga"));

            var model = ConvertWithTextureIndex(dir, scene, "textures\\shared\\cube_e.dds");

            Assert.Contains("env:shared/cube_e.dds", model.LogicalMaterials.Single().Name.Split('|'));
        }

        [Fact]
        public void ConvertToGltf_InlineEnvironmentCubeAbsentFromTheIndex_DropsTheEnvironmentTokens()
        {
            using var dir = new ArchiveTempDir();
            var scene = new NifSceneBuilder();
            var shape = scene.AddShape(scene.Root, "Glass", Triangle, OneTriangle, Uvs);
            scene.AttachShader(shape, NifSceneBuilder.CreateEffectShader(
                Fallout4ShaderPropertyFlags1.Environment_Mapping, 0, envMapTexture: "textures\\shared\\missing_e.dds"));

            var model = ConvertWithTextureIndex(dir, scene, "textures\\shared\\other_d.dds");

            Assert.DoesNotContain("env:", model.LogicalMaterials.Single().Name);
        }

        [Fact]
        public void EnvTextureExists_WithATextureIndexAndNoStagedTextures_UsesTheIndex()
        {
            NifConverter.PreloadTextureIndex(["textures\\shared\\cube_e.dds"]);

            Assert.True(MaterialCensus.EnvTextureExists("shared/cube_e.dds"));
            Assert.False(MaterialCensus.EnvTextureExists("shared/nothing_e.dds"));
        }

        private static ModelRoot ConvertWithTextureIndex(ArchiveTempDir dir, NifSceneBuilder scene, params string[] indexedTextures)
        {
            NifConverter.PreloadTextureIndex(indexedTextures);
            return Convert(dir, scene);
        }

        private static ModelRoot Convert(ArchiveTempDir dir, NifSceneBuilder scene)
        {
            string source = scene.Save(dir.Path, "model.nif");
            string destination = Path.Combine(dir.Path, "model.glb");
            NifConverter.ConvertToGltf(source, destination);
            return ModelRoot.Load(destination);
        }

        private static IList<Vector3> Positions(MeshPrimitive primitive) => primitive.GetVertexAccessor("POSITION").AsVector3Array();

        private static void AssertVector(Vector3 expected, Vector3 actual)
        {
            Assert.Equal(expected.X, actual.X, Tolerance);
            Assert.Equal(expected.Y, actual.Y, Tolerance);
            Assert.Equal(expected.Z, actual.Z, Tolerance);
        }

        private sealed class VectorComparer : IEqualityComparer<Vector3>
        {
            public bool Equals(Vector3 x, Vector3 y) => (x - y).Length() < Tolerance;

            public int GetHashCode(Vector3 obj) => 0;
        }
    }
}
