using System.Numerics;
using NiflySharp.Blocks;

namespace AssetConverter
{
    public readonly record struct TangentStats(int Vertices, int UvGradientVertices, int DegenerateVertices);

    // glTF tangent frame for a converted shape. NIF stores tangent and bitangent
    // swapped relative to the UV axes: the stored Bitangents are +dP/dU and the
    // stored Tangents are +dP/dV. TEXCOORD_0 is written with the NIF's raw V, so
    // both are already expressed against the written UVs. The NIF->glTF axis
    // conversion (x, z, -y) is a proper rotation (det +1), so it neither flips
    // handedness nor needs a w correction; handedness is derived per vertex as
    // w = sign(dot(cross(N, T), dP/dV)).
    internal static class NifTangents
    {
        private const float MinLengthSquared = 1e-12f;

        public static Vector4[] Build(BSTriShape shape, Vector3[] positions, Vector3[] normals,
            Vector2[] uvs, out TangentStats stats)
        {
            int count = positions.Length;
            var result = new Vector4[count];
            var needsFallback = new bool[count];

            var storedTangents = shape.Tangents;
            var storedBitangents = shape.Bitangents;
            bool hasStored = shape.HasTangents
                && storedTangents != null && storedBitangents != null
                && storedTangents.Count == count && storedBitangents.Count == count;

            int fallbackNeeded = 0;
            for (int i = 0; i < count; i++)
            {
                if (hasStored
                    && TryFrame(normals[i], ToGltf(storedBitangents![i]), ToGltf(storedTangents![i]), out var frame))
                    result[i] = frame;
                else
                {
                    needsFallback[i] = true;
                    fallbackNeeded++;
                }
            }

            int degenerate = 0;
            if (fallbackNeeded > 0)
            {
                var (dPdU, dPdV) = AccumulateUvGradients(shape, positions, uvs);
                for (int i = 0; i < count; i++)
                {
                    if (!needsFallback[i]) continue;
                    if (TryFrame(normals[i], dPdU[i], dPdV[i], out var frame))
                        result[i] = frame;
                    else
                    {
                        result[i] = ArbitraryFrame(normals[i]);
                        degenerate++;
                    }
                }
            }

            stats = new TangentStats(count, fallbackNeeded, degenerate);
            return result;
        }

        private static Vector3 ToGltf(Vector3 nif) => new(nif.X, nif.Z, -nif.Y);

        private static bool TryFrame(Vector3 normal, Vector3 dPdU, Vector3 dPdV, out Vector4 frame)
        {
            frame = default;
            var tangent = dPdU - normal * Vector3.Dot(normal, dPdU);
            if (!IsUsable(tangent)) return false;
            tangent = Vector3.Normalize(tangent);

            float handedness = Vector3.Dot(Vector3.Cross(normal, tangent), dPdV);
            if (float.IsNaN(handedness) || handedness == 0f) return false;

            frame = new Vector4(tangent, handedness < 0f ? -1f : 1f);
            return true;
        }

        private static bool IsUsable(Vector3 v) =>
            float.IsFinite(v.X) && float.IsFinite(v.Y) && float.IsFinite(v.Z)
            && v.LengthSquared() > MinLengthSquared;

        private static Vector4 ArbitraryFrame(Vector3 normal)
        {
            var axis = MathF.Abs(normal.Y) < 0.9f ? Vector3.UnitY : Vector3.UnitX;
            var tangent = Vector3.Normalize(Vector3.Cross(axis, normal));
            return new Vector4(tangent, 1f);
        }

        private static (Vector3[] DPdU, Vector3[] DPdV) AccumulateUvGradients(
            BSTriShape shape, Vector3[] positions, Vector2[] uvs)
        {
            var dPdU = new Vector3[positions.Length];
            var dPdV = new Vector3[positions.Length];
            if (shape.Triangles == null) return (dPdU, dPdV);

            foreach (var tri in shape.Triangles)
            {
                int a = tri.V1, b = tri.V2, c = tri.V3;
                if (a >= positions.Length || b >= positions.Length || c >= positions.Length) continue;

                var e1 = positions[b] - positions[a];
                var e2 = positions[c] - positions[a];
                var d1 = uvs[b] - uvs[a];
                var d2 = uvs[c] - uvs[a];

                float det = d1.X * d2.Y - d2.X * d1.Y;
                if (det == 0f || !float.IsFinite(det)) continue;
                float inv = 1f / det;

                var tangent = (e1 * d2.Y - e2 * d1.Y) * inv;
                var bitangent = (e2 * d1.X - e1 * d2.X) * inv;
                foreach (int v in new[] { a, b, c })
                {
                    dPdU[v] += tangent;
                    dPdV[v] += bitangent;
                }
            }
            return (dPdU, dPdV);
        }
    }
}
