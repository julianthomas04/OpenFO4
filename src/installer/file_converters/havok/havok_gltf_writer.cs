using System.Numerics;
using System.Text.Json.Nodes;
using SharpGLTF.Schema2;
using SharpGLTF.Transforms;

namespace AssetConverter.Havok
{
    /// <summary>
    /// Writes Havok skeletons and sampled clips as glTF: bones as nodes, clips as LINEAR animations.
    /// Havok data is in NIF space; everything is converted with <see cref="GameCoordinates"/> so it matches the converted meshes.
    /// </summary>
    public static class HavokGltfWriter
    {
        private const float VectorRestTolerance = 1e-5f;
        private const float RotationRestTolerance = 1e-8f;
        private const string LoopCycle = "loop";

        public static void WriteClipPackGlb(HavokSkeleton skeleton, IReadOnlyList<HavokClip> clips, string path)
        {
            var pack = new ClipPack(skeleton);
            foreach (var clip in clips)
                pack.Add(clip);
            pack.Save(path);
        }

        /// <summary>Accumulates clips one at a time so callers can drop each decoded clip as soon as it is added.</summary>
        public sealed class ClipPack
        {
            private readonly HavokSkeleton _skeleton;
            private readonly ModelRoot _model;
            private readonly Node[] _nodes;

            public ClipPack(HavokSkeleton skeleton)
            {
                _skeleton = skeleton;
                BuildSkeletonModel(skeleton, out _model, out _nodes);
            }

            public int ClipCount => _model.LogicalAnimations.Count;

            public void Add(HavokClip clip) => AddClip(_model, _skeleton, _nodes, clip);

            internal ModelRoot Model => _model;

            public void Save(string path)
            {
                MergeBuffersInPlace();
                _model.SaveGLB(path);
            }

            /// <summary>
            /// GLB holds a single buffer, so <see cref="ModelRoot.SaveGLB"/> deep-clones any model with more than one before merging the clone.
            /// Merging here the same way lets it write this model directly, without the clone's copy of every channel.
            /// </summary>
            private void MergeBuffersInPlace()
            {
                if (_model.LogicalBuffers.Count > 1)
                    _model.MergeBuffers();
            }
        }

        private static void BuildSkeletonModel(HavokSkeleton skeleton, out ModelRoot model, out Node[] nodes)
        {
            RequireUniqueBoneNames(skeleton);

            model = ModelRoot.CreateModel();
            var scene = model.UseScene("skeleton");
            nodes = new Node[skeleton.BoneNames.Count];
            for (int i = 0; i < nodes.Length; i++)
            {
                int parent = skeleton.ParentIndices[i];
                if (parent >= i)
                    throw new HavokFormatException($"Bone {i} '{skeleton.BoneNames[i]}' has parent {parent}; parents must precede children");

                nodes[i] = parent < 0 ? scene.CreateNode(skeleton.BoneNames[i]) : nodes[parent].CreateNode(skeleton.BoneNames[i]);
                var rest = ToGltf(skeleton.ReferencePose[i]);
                nodes[i].LocalTransform = new AffineTransform(rest.Scale, Quaternion.Normalize(rest.Rotation), rest.Translation);
            }
        }

        private static void RequireUniqueBoneNames(HavokSkeleton skeleton)
        {
            var seen = new HashSet<string>(StringComparer.Ordinal);
            foreach (string name in skeleton.BoneNames)
                if (!seen.Add(name))
                    throw new HavokDuplicateBoneNameException(name);
        }

        /// <summary>Throws when the clip cannot be written against the skeleton; lets callers reject a clip before it reaches a shared file.</summary>
        public static void RequireWritable(HavokSkeleton skeleton, HavokClip clip)
        {
            if (!float.IsFinite(clip.FrameDuration) || (clip.FrameCount > 1 && clip.FrameDuration <= 0f))
                throw new HavokFormatException($"Clip '{clip.Name}' has {clip.FrameCount} frames but frame duration {clip.FrameDuration}");
            var driven = new HashSet<int>();
            foreach (var track in clip.Tracks)
            {
                if (track.BoneIndex < 0 || track.BoneIndex >= skeleton.BoneNames.Count)
                    throw new HavokFormatException($"Clip '{clip.Name}' drives bone {track.BoneIndex} but the skeleton has {skeleton.BoneNames.Count}");
                if (!driven.Add(track.BoneIndex))
                    throw new HavokFormatException($"Clip '{clip.Name}' has more than one track for bone {track.BoneIndex} '{skeleton.BoneNames[track.BoneIndex]}'");
                if (track.Frames.Length != clip.FrameCount)
                    throw new HavokFormatException($"Clip '{clip.Name}' track for bone {track.BoneIndex} has {track.Frames.Length} frames, expected {clip.FrameCount}");
            }
        }

        private static void AddClip(ModelRoot model, HavokSkeleton skeleton, Node[] nodes, HavokClip clip)
        {
            RequireWritable(skeleton, clip);

            var animation = model.CreateAnimation(clip.Name);
            foreach (var track in clip.Tracks)
                AddTrack(animation, nodes[track.BoneIndex], ToGltf(skeleton.ReferencePose[track.BoneIndex]), track, clip);
            animation.Extras = BuildExtras(clip);
        }

        private static void AddTrack(Animation animation, Node node, HavokTransform rest, HavokClipTrack track, HavokClip clip)
        {
            var frames = track.Frames.Select(ToGltf).ToArray();
            var translations = frames.Select(f => f.Translation).ToArray();
            var rotations = frames.Select(f => f.Rotation).ToArray();
            var scales = frames.Select(f => f.Scale).ToArray();

            if (!IsConstantAt(translations, rest.Translation, VectorsEqual))
                animation.CreateTranslationChannel(node, Keyframes(translations, clip, VectorsEqual), true);
            if (!IsConstantAt(rotations, rest.Rotation, RotationsEqual))
                animation.CreateRotationChannel(node, Keyframes(rotations, clip, RotationsEqual), true);
            if (!IsConstantAt(scales, rest.Scale, VectorsEqual))
                animation.CreateScaleChannel(node, Keyframes(scales, clip, VectorsEqual), true);
        }

        private static HavokTransform ToGltf(HavokTransform t) => new(
            GameCoordinates.ToGltfPosition(t.Translation),
            GameCoordinates.ToGltfRotation(t.Rotation),
            GameCoordinates.ToGltfScale(t.Scale));

        private static bool IsConstantAt<T>(T[] values, T target, Func<T, T, bool> equal) => values.All(v => equal(v, target));

        /// <summary>All frames, or just first and last key when the channel never changes.</summary>
        private static Dictionary<float, T> Keyframes<T>(T[] values, HavokClip clip, Func<T, T, bool> equal)
        {
            var keys = new Dictionary<float, T>();
            if (IsConstantAt(values, values[0], equal))
            {
                keys[0f] = values[0];
                if (values.Length > 1)
                    keys[(values.Length - 1) * clip.FrameDuration] = values[0];
                return keys;
            }

            for (int f = 0; f < values.Length; f++)
                keys[f * clip.FrameDuration] = values[f];
            return keys;
        }

        private static bool VectorsEqual(Vector3 a, Vector3 b) => (a - b).LengthSquared() <= VectorRestTolerance * VectorRestTolerance;

        private static bool RotationsEqual(Quaternion a, Quaternion b) => 1f - MathF.Abs(Quaternion.Dot(a, b)) <= RotationRestTolerance;

        private static JsonObject BuildExtras(HavokClip clip)
        {
            var annotations = new JsonArray();
            foreach (var annotation in clip.Annotations)
                annotations.Add(new JsonObject { ["t"] = annotation.Time, ["text"] = annotation.Text });

            return new JsonObject
            {
                ["cycle"] = LoopCycle,
                ["annotations"] = annotations,
            };
        }
    }
}
