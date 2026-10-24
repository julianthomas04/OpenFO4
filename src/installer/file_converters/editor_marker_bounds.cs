using System.Numerics;
using NiflySharp;
using NiflySharp.Blocks;
using NiflySharp.Structs;

namespace AssetConverter
{
    /// <summary>
    /// The extent of a model's editor-marker geometry: the shapes whose name contains "EditorMarker", which the game never renders but
    /// uses as volumes (an automatic door's trigger). Coordinates are NIF model space, Bethesda units, Z-up, below the topmost block
    /// (whose own transform the converter ignores as well, whether it is the root node or a shape outside any node).
    /// </summary>
    internal static class EditorMarkerBounds
    {
        const string MarkerName = "EditorMarker";

        /// <exception cref="InvalidDataException">The bytes are not a readable NIF.</exception>
        public static ObjectBoundsJson.FloatBox? Compute(byte[] nifBytes)
        {
            var nif = new NifFile();
            using var stream = new MemoryStream(nifBytes);
            try
            {
                int result = nif.Load(stream);
                if (result != 0) throw new InvalidDataException($"NIF load failed with code {result}");
            }
            catch (Exception ex) when (ex is IOException or ArgumentException or InvalidOperationException)
            {
                throw new InvalidDataException($"NIF load failed: {ex.GetType().Name}: {ex.Message}", ex);
            }
            return Compute(nif);
        }

        /// <returns>The box around every vertex of the editor-marker shapes, or null when the model has none.</returns>
        public static ObjectBoundsJson.FloatBox? Compute(NifFile nif)
        {
            var parents = ParentIndices(nif);
            Vector3? min = null, max = null;
            for (int block = 0; block < nif.Blocks.Count; block++)
            {
                if (nif.Blocks[block] is not BSTriShape shape || shape.VertexPositions is not { Count: > 0 } positions) continue;
                if (shape.Name?.String is not { } name || !name.Contains(MarkerName, StringComparison.OrdinalIgnoreCase)) continue;
                var chain = TransformChain(nif, parents, block);
                foreach (var position in positions)
                {
                    var point = chain.Aggregate(position, (current, node) => Apply(node, current));
                    min = min is { } low ? Vector3.Min(low, point) : point;
                    max = max is { } high ? Vector3.Max(high, point) : point;
                }
            }
            return min is { } lowest && max is { } highest ? new(Rounded(lowest), Rounded(highest)) : null;
        }

        private static Dictionary<int, int> ParentIndices(NifFile nif)
        {
            var parents = new Dictionary<int, int>();
            for (int block = 0; block < nif.Blocks.Count; block++)
            {
                if (nif.Blocks[block] is not NiNode { Children.Indices: { } children }) continue;
                foreach (int child in children)
                    if (child >= 0) parents[child] = block;
            }
            return parents;
        }

        /// <summary>The object's own transform first, then each ancestor's, leaving out the topmost block's.</summary>
        private static List<NiAVObject> TransformChain(NifFile nif, Dictionary<int, int> parents, int block)
        {
            var chain = new List<NiAVObject>();
            int current = block;
            while (true)
            {
                chain.Add((NiAVObject)nif.Blocks[current]);
                if (chain.Count > nif.Blocks.Count) throw new InvalidDataException("the node hierarchy contains a cycle");
                if (!parents.TryGetValue(current, out int parent)) break;
                current = parent;
            }
            chain.RemoveAt(chain.Count - 1);
            return chain;
        }

        /// <summary>Scale, then rotate (the NIF matrix acts on row vectors, as <see cref="GameCoordinates.ToGltfTransform"/> reads it), then translate.</summary>
        private static Vector3 Apply(NiAVObject node, Vector3 point)
        {
            var m = node.Rotation;
            var scaled = point * node.Scale;
            var rotated = new Vector3(
                scaled.X * m.M11 + scaled.Y * m.M21 + scaled.Z * m.M31,
                scaled.X * m.M12 + scaled.Y * m.M22 + scaled.Z * m.M32,
                scaled.X * m.M13 + scaled.Y * m.M23 + scaled.Z * m.M33);
            return rotated + node.Translation;
        }

        private static ObjectBoundsJson.FloatPoint Rounded(Vector3 v) => new(Round(v.X), Round(v.Y), Round(v.Z));

        private static float Round(float value) => MathF.Round(value, 3);
    }
}
