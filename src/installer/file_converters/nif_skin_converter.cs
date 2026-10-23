using System.Numerics;
using NiflySharp;
using NiflySharp.Blocks;
using NiflySharp.Structs;
using SharpGLTF.Materials;
using SharpGLTF.Scenes;
using SharpGLTF.Schema2;

namespace AssetConverter
{
    // Converts FO4 skinned shapes (BSSkin::Instance + BSSkin::BoneData).
    //
    // Skin space convention: the NIF converter resets the root NiNode transform to
    // identity (the engine ignores it at placement) and keeps every other node's
    // local transform, so shape vertices stay in root-relative space exactly like
    // rigid shapes. Inverse bind matrices are the NIF skin-to-bone transforms run
    // through GameCoordinates unchanged: jointWorld * inverseBind maps skin space to world,
    // and for architecture whose bones carry their bind pose it is the identity.
    // Actor NIFs ship their bones as pose-less NiNodes; the engine binds joints to
    // the real skeleton by name and only needs the inverse binds, which stay valid.
    //
    // A shape whose every vertex is weighted to exactly one bone, and whose bones
    // include one that a sequence of the same file animates, is "rigidified": split
    // per bone, moved into the bone's local space and attached under the bone's
    // glTF node, so the transform animation drives the mesh with no skinning at
    // runtime. Without an animated bone the file gives no reason to detach the mesh
    // from its skeleton (actor limb caps, wings), so it stays a true glTF skin.
    internal static class NifSkinConverter
    {
        private const float SingleBoneWeight = 0.999f;

        public static void Add(
            BSTriShape shape, string shapeName, ShapeVertexData vertices, MaterialBuilder material,
            NifFile nif, SceneBuilder scene, Dictionary<int, NodeBuilder> nodeMap, ISet<string> animatedNodeNames)
        {
            var skin = ReadSkin(shape, nif, nodeMap);
            var influences = ReadInfluences(shape, skin.Bones.Length);
            ValidateTriangles(shape.Triangles, influences.Length);

            if (IsRigidlyAnimated(skin, influences, animatedNodeNames))
            {
                AddRigidified(shape, shapeName, vertices, material, scene, skin, influences);
                NifAnimationStats.Add(NifAnimationStats.SkinsRigidified);
            }
            else
            {
                AddTrueSkin(shapeName, vertices, material, scene, skin, influences, shape.Triangles);
                NifAnimationStats.Add(NifAnimationStats.SkinsTrueSkinned);
            }
        }

        // glTF requires a name on every node the engine imports; SharpGLTF leaves
        // the instance node of a skinned mesh unnamed.
        public static void NameSkinnedNodes(ModelRoot model)
        {
            foreach (var node in model.LogicalNodes.Where(n => n.Skin != null && string.IsNullOrEmpty(n.Name)))
                node.Name = node.Mesh?.Name is { Length: > 0 } meshName ? meshName : $"skinned_{node.LogicalIndex}";
        }

        private sealed record SkinBinding(NodeBuilder[] Bones, Matrix4x4[] InverseBind);

        private static SkinBinding ReadSkin(BSTriShape shape, NifFile nif, Dictionary<int, NodeBuilder> nodeMap)
        {
            var boneIndices = BsSkinBlockReader.ReadInstanceBones(nif, shape.SkinInstanceRef.Index, out int boneDataRef);
            if (boneDataRef < 0)
                throw new NifSkinException("skin instance has no BSSkin::BoneData");
            var transforms = BsSkinBlockReader.ReadBoneTransforms(nif, boneDataRef);
            if (boneIndices.Length != transforms.Length)
                throw new NifSkinException($"skin has {boneIndices.Length} bones but {transforms.Length} bind transforms");

            var bones = new NodeBuilder[boneIndices.Length];
            for (int i = 0; i < bones.Length; i++)
            {
                if (!nodeMap.TryGetValue(boneIndices[i], out var bone))
                    throw new NifSkinException($"bone {i} (block {boneIndices[i]}) is not a NiNode of this file");
                bones[i] = bone;
            }
            var inverseBind = transforms.Select(b => GameCoordinates.ToGltfTransform(b.Rotation, b.Translation, b.Scale)).ToArray();
            return new SkinBinding(bones, inverseBind);
        }

        private static (int Bone, float Weight)[][] ReadInfluences(BSTriShape shape, int boneCount)
        {
            var vertexData = shape.VertexData;
            if (vertexData == null || vertexData.Count != shape.VertexPositions.Count)
                throw new NifSkinException("skinned shape has no per-vertex bone data");

            var result = new (int, float)[vertexData.Count][];
            for (int v = 0; v < result.Length; v++)
            {
                var weights = vertexData[v].BoneWeights;
                var indices = vertexData[v].BoneIndices;
                var merged = new Dictionary<int, float>();
                for (int k = 0; k < 4; k++)
                {
                    float weight = (float)weights[k];
                    if (weight <= 0f)
                        continue;
                    if (indices[k] >= boneCount)
                        throw new NifSkinException($"vertex {v} references bone {indices[k]} of {boneCount}");
                    merged[indices[k]] = merged.GetValueOrDefault(indices[k]) + weight;
                }
                result[v] = Normalize(merged, v);
            }
            return result;
        }

        private static void ValidateTriangles(IEnumerable<Triangle> triangles, int vertexCount)
        {
            foreach (var tri in triangles)
            {
                if (tri.V1 >= vertexCount || tri.V2 >= vertexCount || tri.V3 >= vertexCount)
                    throw new NifSkinException($"triangle ({tri.V1}, {tri.V2}, {tri.V3}) references a vertex outside the {vertexCount} of the shape");
            }
        }

        private static (int Bone, float Weight)[] Normalize(Dictionary<int, float> weights, int vertex)
        {
            float total = weights.Values.Sum();
            if (total <= 0f)
                throw new NifSkinException($"vertex {vertex} has no bone weight");
            var ordered = weights.OrderByDescending(kv => kv.Value).Select(kv => (kv.Key, kv.Value / total)).ToArray();
            return ordered[0].Item2 >= SingleBoneWeight ? new[] { (ordered[0].Key, 1f) } : ordered;
        }

        private static void AddRigidified(
            BSTriShape shape, string shapeName, ShapeVertexData vertices, MaterialBuilder material,
            SceneBuilder scene, SkinBinding skin, (int Bone, float Weight)[][] influences)
        {
            var trianglesByBone = new SortedDictionary<int, List<Triangle>>();
            foreach (var tri in shape.Triangles)
            {
                int bone = influences[tri.V1][0].Bone;
                if (influences[tri.V2][0].Bone != bone || influences[tri.V3][0].Bone != bone)
                    NifAnimationStats.Add(NifAnimationStats.SkinStraddlingTriangles);
                if (!trianglesByBone.TryGetValue(bone, out var list))
                    trianglesByBone[bone] = list = new List<Triangle>();
                list.Add(tri);
            }

            foreach (var (bone, triangles) in trianglesByBone)
            {
                var local = ToBoneSpace(vertices, skin.InverseBind[bone], out bool mirrored);
                var oriented = mirrored ? triangles.Select(t => new Triangle(t.V1, t.V3, t.V2)).ToList() : triangles;
                string meshName = trianglesByBone.Count == 1 ? shapeName : $"{shapeName}:{skin.Bones[bone].Name}";
                var mesh = NifMeshBuilder.Rigid(meshName, local, oriented, material);
                var meshNode = new NodeBuilder(meshName);
                skin.Bones[bone].AddNode(meshNode);
                scene.AddRigidMesh(mesh, meshNode);
            }
        }

        // Positions and tangents follow the matrix; normals follow its inverse transpose so
        // non-uniform scale keeps them perpendicular. A mirroring matrix (negative
        // determinant) flips triangle winding and the tangent handedness.
        private static ShapeVertexData ToBoneSpace(ShapeVertexData skinSpace, Matrix4x4 skinToBone, out bool mirrored)
        {
            if (!Matrix4x4.Invert(skinToBone, out var inverse))
                throw new NifSkinException("skin-to-bone transform is not invertible");
            var normalMatrix = Matrix4x4.Transpose(inverse);
            mirrored = skinToBone.GetDeterminant() < 0f;
            float handedness = mirrored ? -1f : 1f;

            int count = skinSpace.Positions.Length;
            var positions = new Vector3[count];
            var normals = new Vector3[count];
            var tangents = new Vector4[count];
            for (int i = 0; i < count; i++)
            {
                positions[i] = Vector3.Transform(skinSpace.Positions[i], skinToBone);
                normals[i] = Vector3.Normalize(Vector3.TransformNormal(skinSpace.Normals[i], normalMatrix));
                var t = Vector3.Normalize(Vector3.TransformNormal(new Vector3(skinSpace.Tangents[i].X, skinSpace.Tangents[i].Y, skinSpace.Tangents[i].Z), skinToBone));
                tangents[i] = new Vector4(t, skinSpace.Tangents[i].W * handedness);
            }
            return new ShapeVertexData(positions, normals, tangents, skinSpace.TexCoords, skinSpace.Colors);
        }

        private static void AddTrueSkin(
            string shapeName, ShapeVertexData vertices, MaterialBuilder material, SceneBuilder scene,
            SkinBinding skin, (int Bone, float Weight)[][] influences, IEnumerable<Triangle> triangles)
        {
            var mesh = NifMeshBuilder.Skinned(shapeName, vertices, influences, triangles, material);
            var joints = skin.Bones.Select((bone, i) => (bone, skin.InverseBind[i])).ToArray();
            scene.AddSkinnedMesh(mesh, joints);
        }

        private static bool IsRigidlyAnimated(
            SkinBinding skin, (int Bone, float Weight)[][] influences, ISet<string> animatedNodeNames)
            => influences.All(i => i.Length == 1)
               && influences.Select(i => i[0].Bone).Distinct().Any(bone => animatedNodeNames.Contains(skin.Bones[bone].Name));
    }
}
