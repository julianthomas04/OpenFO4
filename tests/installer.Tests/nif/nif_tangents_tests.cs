using System.Numerics;
using AssetConverter;
using NiflySharp;
using NiflySharp.Blocks;
using NiflySharp.Structs;
using Xunit;

namespace InstallerTests.Nif
{
    // Convention: nif_tangents.cs header comment. NIF stores Bitangents = +dP/dU and Tangents = +dP/dV; the glTF tangent is
    // the unit vector along dP/dU orthogonalised against the normal, w = sign(dot(cross(N, T), dP/dV)).
    public class NifTangentsTests
    {
        private const float Tolerance = 1e-5f;
        private const float ByteQuantisationTolerance = 0.02f; // stored NIF tangents are packed into one byte per component

        // glTF-space triangle in the XZ plane with the normal pointing +Y.
        private static readonly Vector3[] FlatPositions = [new(0, 0, 0), new(2, 0, 0), new(0, 0, -2)];
        private static readonly Vector3[] UpNormals = [Vector3.UnitY, Vector3.UnitY, Vector3.UnitY];
        private static readonly Vector2[] UnmirroredUvs = [new(0, 0), new(1, 0), new(0, 1)];
        private static readonly Vector2[] MirroredUvs = [new(0, 0), new(-1, 0), new(0, 1)];

        [Fact]
        public void Build_StoredTangentFrame_IsConvertedToGltfAxesWithPositiveHandedness()
        {
            // NIF space: dP/dU = +X, dP/dV = +Y, normal +Z. In glTF (x, z, -y): dP/dU = +X, dP/dV = -Z, normal +Y.
            var shape = Shape(withStoredTangents: [Vector3.UnitY, Vector3.UnitY, Vector3.UnitY], bitangents: [Vector3.UnitX, Vector3.UnitX, Vector3.UnitX]);

            var frames = NifTangents.Build(shape, FlatPositions, UpNormals, UnmirroredUvs, out var stats);

            AssertFrame(new Vector4(1f, 0f, 0f, 1f), frames[0], ByteQuantisationTolerance);
            Assert.Equal(new TangentStats(3, 0, 0), stats);
        }

        [Fact]
        public void Build_StoredTangentFrameWithFlippedV_GetsNegativeHandedness()
        {
            var shape = Shape(withStoredTangents: [-Vector3.UnitY, -Vector3.UnitY, -Vector3.UnitY], bitangents: [Vector3.UnitX, Vector3.UnitX, Vector3.UnitX]);

            var frames = NifTangents.Build(shape, FlatPositions, UpNormals, UnmirroredUvs, out _);

            AssertFrame(new Vector4(1f, 0f, 0f, -1f), frames[1], ByteQuantisationTolerance);
        }

        [Fact]
        public void Build_StoredTangentNotPerpendicularToTheNormal_IsOrthogonalisedAgainstIt()
        {
            // Stored dP/dU = (1, 0, 1) in NIF space -> glTF (1, 1, 0); against normal +Y the tangent must lose its Y part.
            var shape = Shape(withStoredTangents: [Vector3.UnitY, Vector3.UnitY, Vector3.UnitY],
                bitangents: [new Vector3(1, 0, 1), new Vector3(1, 0, 1), new Vector3(1, 0, 1)]);

            var frames = NifTangents.Build(shape, FlatPositions, UpNormals, UnmirroredUvs, out _);

            AssertFrame(new Vector4(1f, 0f, 0f, 1f), frames[0], ByteQuantisationTolerance);
        }

        [Fact]
        public void Build_NoStoredTangents_DerivesThemFromTheUvGradients()
        {
            var shape = Shape(withStoredTangents: null);

            var frames = NifTangents.Build(shape, FlatPositions, UpNormals, UnmirroredUvs, out var stats);

            AssertFrame(new Vector4(1f, 0f, 0f, 1f), frames[0]);
            AssertFrame(new Vector4(1f, 0f, 0f, 1f), frames[2]);
            Assert.Equal(new TangentStats(3, 3, 0), stats);
        }

        [Fact]
        public void Build_MirroredUvs_FlipTheTangentDirectionAndHandedness()
        {
            var shape = Shape(withStoredTangents: null);

            var frames = NifTangents.Build(shape, FlatPositions, UpNormals, MirroredUvs, out _);

            AssertFrame(new Vector4(-1f, 0f, 0f, -1f), frames[0]);
        }

        [Fact]
        public void Build_UvGradientScale_DoesNotChangeTheUnitTangent()
        {
            var shape = Shape(withStoredTangents: null);
            Vector2[] stretched = [new(0, 0), new(40, 0), new(0, 0.25f)];

            var frames = NifTangents.Build(shape, FlatPositions, UpNormals, stretched, out _);

            AssertFrame(new Vector4(1f, 0f, 0f, 1f), frames[0]);
        }

        [Fact]
        public void Build_DegenerateUvs_FallBackToAnArbitraryFrameAndCountThem()
        {
            var shape = Shape(withStoredTangents: null);
            Vector2[] collapsed = [new(0.5f, 0.5f), new(0.5f, 0.5f), new(0.5f, 0.5f)];

            var frames = NifTangents.Build(shape, FlatPositions, UpNormals, collapsed, out var stats);

            // Normal +Y is steeper than 0.9, so the helper axis is +X and the tangent is cross(X, N) = +Z.
            AssertFrame(new Vector4(0f, 0f, 1f, 1f), frames[0]);
            Assert.Equal(new TangentStats(3, 3, 3), stats);
        }

        [Fact]
        public void Build_NormalAlongZWithDegenerateUvs_UsesTheYAxisAsHelper()
        {
            var shape = Shape(withStoredTangents: null);
            Vector3[] normals = [Vector3.UnitZ, Vector3.UnitZ, Vector3.UnitZ];
            Vector2[] collapsed = [Vector2.Zero, Vector2.Zero, Vector2.Zero];

            var frames = NifTangents.Build(shape, FlatPositions, normals, collapsed, out _);

            AssertFrame(new Vector4(1f, 0f, 0f, 1f), frames[0]);
        }

        [Fact]
        public void Build_VertexOnlyUsedByAnUnusableTriangle_IsDegenerateWhileOthersKeepTheirUvTangent()
        {
            // Second triangle repeats one UV so its gradient determinant is zero; its extra vertex (3) gets no gradient.
            var shape = Shape(withStoredTangents: null, extraVertexAndTriangle: true);
            Vector3[] positions = [.. FlatPositions, new Vector3(2, 0, -2)];
            Vector3[] normals = [.. UpNormals, Vector3.UnitY];
            Vector2[] uvs = [.. UnmirroredUvs, new Vector2(0, 1)];

            var frames = NifTangents.Build(shape, positions, normals, uvs, out var stats);

            AssertFrame(new Vector4(1f, 0f, 0f, 1f), frames[0]);
            Assert.Equal(1, stats.DegenerateVertices);
        }

        [Fact]
        public void Build_RandomTriangles_AlwaysYieldsAnOrthonormalFrameWithUnitHandedness()
        {
            var random = new Random(1234);
            for (int i = 0; i < 300; i++)
            {
                var shape = Shape(withStoredTangents: null);
                var positions = Enumerable.Range(0, 3).Select(_ => RandomVector(random, 5f)).ToArray();
                var normals = Enumerable.Range(0, 3).Select(_ => Vector3.Normalize(RandomVector(random, 1f) + Vector3.UnitX * 0.01f)).ToArray();
                var uvs = Enumerable.Range(0, 3).Select(_ => new Vector2(random.NextSingle(), random.NextSingle())).ToArray();

                var frames = NifTangents.Build(shape, positions, normals, uvs, out _);

                for (int v = 0; v < 3; v++)
                {
                    var tangent = new Vector3(frames[v].X, frames[v].Y, frames[v].Z);
                    Assert.Equal(1f, tangent.Length(), 1e-4f);
                    Assert.Equal(0f, Vector3.Dot(tangent, normals[v]), 1e-4f);
                    Assert.True(frames[v].W is 1f or -1f, $"w was {frames[v].W}");
                }
            }
        }

        private static Vector3 RandomVector(Random random, float range) =>
            new((random.NextSingle() - 0.5f) * range, (random.NextSingle() - 0.5f) * range, (random.NextSingle() - 0.5f) * range);

        /// <summary>A BSTriShape with the stored tangents either removed (null) or set from NIF-space tangent / bitangent lists.</summary>
        private static BSTriShape Shape(Vector3[]? withStoredTangents, Vector3[]? bitangents = null, bool extraVertexAndTriangle = false)
        {
            var positions = new List<Vector3> { new(0, 0, 0), new(2, 0, 0), new(0, 2, 0) };
            var triangles = new List<Triangle> { new(0, 1, 2) };
            var uvs = new List<TexCoord> { new(0, 0), new(1, 0), new(0, 1) };
            var normals = new List<Vector3> { Vector3.UnitZ, Vector3.UnitZ, Vector3.UnitZ };
            if (extraVertexAndTriangle)
            {
                positions.Add(new Vector3(2, 2, 0));
                uvs.Add(new TexCoord(0, 1));
                normals.Add(Vector3.UnitZ);
                triangles.Add(new Triangle(1, 3, 2));
            }

            var shape = new BSTriShape();
            shape.Create(NiVersion.GetFO4(), positions, triangles, uvs, normals);
            if (withStoredTangents is null)
            {
                shape.HasTangents = false;
            }
            else
            {
                shape.SetTangents(withStoredTangents.ToList());
                shape.SetBitangents((bitangents ?? withStoredTangents).ToList());
            }
            return shape;
        }

        private static void AssertFrame(Vector4 expected, Vector4 actual, float tolerance = Tolerance)
        {
            Assert.Equal(expected.X, actual.X, tolerance);
            Assert.Equal(expected.Y, actual.Y, tolerance);
            Assert.Equal(expected.Z, actual.Z, tolerance);
            Assert.Equal(expected.W, actual.W);
        }
    }
}
