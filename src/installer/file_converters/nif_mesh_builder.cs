using System.Numerics;
using NiflySharp.Structs;
using SharpGLTF.Geometry;
using SharpGLTF.Geometry.VertexTypes;
using SharpGLTF.Materials;

namespace AssetConverter
{
    // Per-vertex attributes of one shape, already in glTF space. Colors carries the NIF
    // vertex colours (RGB tint, A weight) or white with zero alpha when the shape has none.
    internal readonly record struct ShapeVertexData(
        Vector3[] Positions,
        Vector3[] Normals,
        Vector4[] Tangents,
        Vector2[] TexCoords,
        Vector4[] Colors);

    internal static class NifMeshBuilder
    {
        public static IMeshBuilder<MaterialBuilder> Rigid(
            string meshName, ShapeVertexData data, IEnumerable<Triangle> triangles, MaterialBuilder material)
        {
            var meshBuilder = new MeshBuilder<VertexPositionNormalTangent, VertexColor1Texture1>(meshName);
            var primitive = meshBuilder.UsePrimitive(material);
            var vertices = new VertexBuilder<VertexPositionNormalTangent, VertexColor1Texture1, VertexEmpty>[data.Positions.Length];
            for (int i = 0; i < vertices.Length; i++)
                vertices[i] = new(Geometry(data, i), new VertexColor1Texture1(data.Colors[i], data.TexCoords[i]));
            AddTriangles(primitive, vertices, triangles);
            return meshBuilder;
        }

        public static IMeshBuilder<MaterialBuilder> Points(string meshName, IEnumerable<Vector3> positions, MaterialBuilder material)
        {
            var meshBuilder = new MeshBuilder<VertexPosition>(meshName);
            var primitive = meshBuilder.UsePrimitive(material, 1);
            foreach (var position in positions)
                primitive.AddPoint(new VertexBuilder<VertexPosition, VertexEmpty, VertexEmpty>(new VertexPosition(position)));
            return meshBuilder;
        }

        public static IMeshBuilder<MaterialBuilder> Skinned(
            string meshName, ShapeVertexData data, (int Joint, float Weight)[][] influences,
            IEnumerable<Triangle> triangles, MaterialBuilder material)
        {
            var meshBuilder = new MeshBuilder<VertexPositionNormalTangent, VertexColor1Texture1, VertexJoints4>(meshName);
            var primitive = meshBuilder.UsePrimitive(material);
            var vertices = new VertexBuilder<VertexPositionNormalTangent, VertexColor1Texture1, VertexJoints4>[data.Positions.Length];
            for (int i = 0; i < vertices.Length; i++)
                vertices[i] = new(Geometry(data, i), new VertexColor1Texture1(data.Colors[i], data.TexCoords[i]), new VertexJoints4(influences[i]));
            AddTriangles(primitive, vertices, triangles);
            return meshBuilder;
        }

        private static VertexPositionNormalTangent Geometry(ShapeVertexData data, int index)
            => new(data.Positions[index], data.Normals[index], data.Tangents[index]);

        private static void AddTriangles<TvG, TvM, TvS>(
            PrimitiveBuilder<MaterialBuilder, TvG, TvM, TvS> primitive,
            VertexBuilder<TvG, TvM, TvS>[] vertices,
            IEnumerable<Triangle> triangles)
            where TvG : struct, IVertexGeometry
            where TvM : struct, IVertexMaterial
            where TvS : struct, IVertexSkinning
        {
            foreach (var tri in triangles)
            {
                if (tri.V1 < vertices.Length && tri.V2 < vertices.Length && tri.V3 < vertices.Length)
                    primitive.AddTriangle(vertices[tri.V1], vertices[tri.V2], vertices[tri.V3]);
            }
        }
    }
}
