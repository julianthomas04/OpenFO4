using System.Numerics;
using System.Text.Json.Nodes;
using NiflySharp;
using NiflySharp.Blocks;
using SharpGLTF.Scenes;

namespace AssetConverter
{
    internal sealed record LightExtrasResult(JsonObject? Extras, IReadOnlyList<string> Unsupported);

    // Builds the node-level glTF extras of marker and light models: `addon_node` for a BSValueNode and
    // `light` for a NiPointLight (field names and units follow the engine's model_extras.rs contract).
    // Lengths are glTF-space (game units x GameCoordinates.Scaling). The light's `offset` track is the
    // light's model-space position minus its rest position while the transform animation of its
    // ancestors plays (a controller sequence, else the ancestors' own NiTransformControllers), sampled at NifKeySampler.SampleRate through the full parent chain so the
    // engine needs no node animation. The root node is skipped in that chain because the converter
    // strips its transform.
    internal static class NifNodeExtras
    {
        public const int Version = 1;
        public const string OffsetTarget = "offset";
        public const string PointKind = "point";

        public const string OffsetTiesCounter = "light offset chains driven by several equally ranked sequences (first in file order used)";
        public const string OffsetDroppedStandaloneCounter = "light offset standalone transform controllers dropped (other timing or duplicate node)";
        public const string OffsetAllZeroCounter = "light offset chains with no movement (offset track omitted)";

        private const float MovementEpsilon = 1e-6f;
        private const string StandaloneClipName = "standalone transform controllers";

        public const string HiddenKey = "hidden";
        public const string DamageStageKey = "damage_stage";
        private const uint AppCulledFlag = 1u;

        // Own state of one NIF object only: `hidden` when its APP_CULLED bit is set, `damage_stage` for the
        // range nodes that gate damage-dependent attachments. Descendants of a hidden node are not marked;
        // the engine inherits by walking ancestors.
        public static void ApplyObjectState(NiAVObject source, NodeBuilder node)
        {
            bool hidden = (source.Flags_ui & AppCulledFlag) != 0;
            bool stage = source is BSBlastNode;
            if (!hidden && !stage)
                return;

            var extras = GltfNodeExtras.Existing(node.Extras);
            if (hidden)
                extras[HiddenKey] = true;
            if (source is BSBlastNode range)
                extras[DamageStageKey] = new JsonObject { ["min"] = (int)range.Min, ["max"] = (int)range.Max };
            node.Extras = extras;
        }

        public const string ConnectParentsKey = "connect_parents";
        public const string ConnectChildrenKey = "connect_children";
        public const string UnreadableConnectPointBlockCounter = "connect point blocks unreadable";
        public const string UnresolvedConnectParentCounter = "connect points dropped because their parent node does not exist";

        // Weapon assembly attach data. `connect_parents` lands on the node named by each point's parent
        // (the root when empty) with the point's pose local to that node; `connect_children` lands on the root
        // together with the root's own transform, which the converter strips from the glTF root.
        public static void ApplyConnectPoints(NifFile nif, IReadOnlyDictionary<int, NodeBuilder> nodeMap, int rootBlock, Action<string> warn)
        {
            var parentBlocks = BsConnectPointBlockReader.BlocksOfType(nif, BsConnectPointBlockReader.ParentsTypeName).ToList();
            var childBlocks = BsConnectPointBlockReader.BlocksOfType(nif, BsConnectPointBlockReader.ChildrenTypeName).ToList();
            if (parentBlocks.Count == 0 && childBlocks.Count == 0)
                return;

            var blockByName = new Dictionary<string, int>(StringComparer.Ordinal);
            foreach (var block in nodeMap.Keys.OrderBy(b => b))
                blockByName.TryAdd(NifNodeNames.Of(nif, block), block);

            foreach (int block in parentBlocks)
            {
                if (!TryRead(() => BsConnectPointBlockReader.ReadParents(nif, block), block, warn, out var points))
                    continue;
                foreach (var point in points)
                    AddParentPoint(nodeMap, blockByName, rootBlock, point, warn);
            }

            foreach (int block in childBlocks)
            {
                if (!TryRead(() => BsConnectPointBlockReader.ReadChildren(nif, block), block, warn, out var children))
                    continue;
                var extras = GltfNodeExtras.Existing(nodeMap[rootBlock].Extras);
                extras[ConnectChildrenKey] = new JsonObject
                {
                    ["v"] = Version,
                    ["names"] = new JsonArray(children.Names.Select(n => (JsonNode?)JsonValue.Create(n)).ToArray()),
                    ["skinned"] = children.Skinned,
                    ["root"] = PoseJson(RestPose((NiAVObject)nif.Blocks[rootBlock])),
                };
                nodeMap[rootBlock].Extras = extras;
            }
        }

        private static bool TryRead<T>(Func<T> read, int block, Action<string> warn, out T value)
        {
            try
            {
                value = read();
                return true;
            }
            catch (Exception ex) when (ex is NifConnectPointException or EndOfStreamException)
            {
                NifAnimationStats.Add(UnreadableConnectPointBlockCounter);
                warn($"connect point block {block} unreadable, dropped: {ex.Message}");
                value = default!;
                return false;
            }
        }

        private static void AddParentPoint(IReadOnlyDictionary<int, NodeBuilder> nodeMap, Dictionary<string, int> blockByName, int rootBlock,
            BsConnectPointBlockReader.ParentPoint point, Action<string> warn)
        {
            int target = rootBlock;
            if (point.Parent.Length > 0 && !blockByName.TryGetValue(point.Parent, out target))
            {
                NifAnimationStats.Add(UnresolvedConnectParentCounter);
                warn($"connect point '{point.Name}' names parent node '{point.Parent}' which does not exist; dropped");
                return;
            }
            var node = nodeMap[target];
            var extras = GltfNodeExtras.Existing(node.Extras);
            if ((extras[ConnectParentsKey] as JsonObject)?["points"] is not JsonArray points)
            {
                points = new JsonArray();
                extras[ConnectParentsKey] = new JsonObject { ["v"] = Version, ["points"] = points };
            }
            var rotation = GameCoordinates.ToGltfRotation(point.Rotation);
            points.Add(PoseJson(new LocalPose(GameCoordinates.ToGltfPosition(point.Translation), rotation, new Vector3(point.Scale)), point.Name));
            node.Extras = extras;
        }

        private static JsonObject PoseJson(LocalPose pose, string? name = null)
        {
            var json = new JsonObject();
            if (name != null)
                json["name"] = name;
            json["translation"] = Vector(pose.Translation);
            json["rotation"] = new JsonArray(new[] { pose.Rotation.X, pose.Rotation.Y, pose.Rotation.Z, pose.Rotation.W }.Select(c => (JsonNode?)JsonValue.Create(c)).ToArray());
            json["scale"] = pose.Scale.X;
            return json;
        }

        private static JsonArray Vector(Vector3 v) =>
            new(new[] { v.X, v.Y, v.Z }.Select(c => (JsonNode?)JsonValue.Create(c)).ToArray());

        public static JsonObject AddonNode(NifFile nif, int block)
        {
            if (!NifControllerExtractor.TryGetBlock<BSValueNode>(nif, block, "addon node", out var node))
                throw new NifAnimationException($"block {block} is not a BSValueNode");
            return new JsonObject
            {
                ["v"] = Version,
                ["index"] = checked((int)node.Value),
                ["flags"] = (uint)node.ValueNodeFlags,
            };
        }

        // Extras is null when the light has nothing to animate; unsupported content is counted in
        // NifAnimationStats and listed in the result.
        public static LightExtrasResult Light(NifFile nif, int block, IReadOnlyList<SequenceClip> clips, IReadOnlyDictionary<int, int>? parents = null)
        {
            if (!NifControllerExtractor.TryGetBlock<NiLight>(nif, block, "light", out var light))
                throw new NifAnimationException($"block {block} is not a NiLight");
            if (light.GetType() != typeof(NiPointLight))
            {
                string description = $"light block {light.GetType().Name}";
                NifAnimationStats.AddUnsupported(description);
                return new LightExtrasResult(null, new[] { description });
            }

            var controllers = ShaderControllerTracks.ForLight(nif, block);
            var tracks = controllers.Tracks.ToList();
            var offset = OffsetTrack(nif, block, light, clips, parents ?? NifParentMap.Build(nif));
            if (offset != null)
                tracks.Add(offset);
            if (tracks.Count == 0)
                return new LightExtrasResult(null, controllers.Unsupported);

            var extras = new JsonObject
            {
                ["v"] = Version,
                ["kind"] = PointKind,
                ["dimmer_rest"] = light.Dimmer,
                ["tracks"] = new JsonArray(tracks.Select(TrackJson).ToArray<JsonNode?>()),
            };
            return new LightExtrasResult(extras, controllers.Unsupported);
        }

        private static JsonObject TrackJson(ControllerTrack track)
        {
            bool scalar = track.Target != ShaderControllerTracks.LightColorTarget && track.Target != OffsetTarget;
            var values = track.Values
                .Select(v => scalar ? JsonValue.Create(v[0]) : (JsonNode)new JsonArray(v.Select(c => (JsonNode?)JsonValue.Create(c)).ToArray()))
                .ToArray<JsonNode?>();
            return new JsonObject
            {
                ["target"] = track.Target,
                ["cycle"] = track.Cycle,
                ["start"] = track.Start,
                ["stop"] = track.Stop,
                ["frequency"] = track.Frequency,
                ["phase"] = track.Phase,
                ["times"] = new JsonArray(track.Times.Select(t => (JsonNode?)JsonValue.Create(t)).ToArray()),
                ["values"] = new JsonArray(values),
            };
        }

        private readonly record struct LocalPose(Vector3 Translation, Quaternion Rotation, Vector3 Scale)
        {
            public Matrix4x4 ToMatrix() => Matrix4x4.CreateScale(Scale) * Matrix4x4.CreateFromQuaternion(Rotation) * Matrix4x4.CreateTranslation(Translation);
        }

        private sealed record ChainNode(string? Name, LocalPose Rest);

        private static ControllerTrack? OffsetTrack(NifFile nif, int lightBlock, NiLight light, IReadOnlyList<SequenceClip> clips, IReadOnlyDictionary<int, int> parents)
        {
            var ancestors = Ancestors(nif, parents, lightBlock).ToList();
            var chain = new List<ChainNode> { new(null, RestPose(light)) };
            foreach (var ancestor in ancestors)
                chain.Add(new ChainNode(ancestor.Name?.String, RestPose(ancestor)));
            var names = chain.Select(c => c.Name).OfType<string>().Where(n => n.Length > 0).ToHashSet(StringComparer.Ordinal);

            var clip = PickClip(clips, names) ?? StandaloneClip(nif, ancestors);
            if (clip == null)
                return null;

            var animated = clip.Tracks.Where(t => t.HasKeys).ToDictionary(t => t.NodeName, StringComparer.Ordinal);
            NodeTrack? TrackOf(string? name) => name != null && animated.TryGetValue(name, out var track) ? track : null;
            var restPosition = ChainMatrix(chain, _ => null, 0f).Translation;
            var times = SampleTimes(clip.Start, clip.Stop);
            var offsets = times.Select(t => ChainMatrix(chain, TrackOf, t).Translation - restPosition).ToList();
            if (offsets.All(o => o.Length() < MovementEpsilon))
            {
                NifAnimationStats.Add(OffsetAllZeroCounter);
                return null;
            }
            return new ControllerTrack(OffsetTarget, clip.Cycle, clip.Start, clip.Stop, clip.Frequency, clip.Phase, times, offsets.Select(o => new[] { o.X, o.Y, o.Z }).ToList());
        }

        private static SequenceClip? PickClip(IReadOnlyList<SequenceClip> clips, HashSet<string> chainNames)
        {
            var candidates = clips
                .Where(c => !c.PoseOnly && c.Tracks.Any(t => t.HasKeys && chainNames.Contains(t.NodeName)))
                .ToList();
            return LoopFirstRanking.Pick(candidates, c => c.Cycle == NifControllerExtractor.CycleLoop, OffsetTiesCounter);
        }

        // Ancestors driven by a NiTransformController of their own (no controller sequence): the controllers that share a
        // cycle type, frequency and phase form one clip; the looping group wins and the others are dropped and counted.
        private static SequenceClip? StandaloneClip(NifFile nif, IReadOnlyList<NiNode> ancestors)
        {
            var drivers = ancestors.SelectMany(a => StandaloneTransformControllers.On(nif, a)).ToList();
            var groups = drivers.GroupBy(d => (d.Timing.Cycle, d.Timing.Frequency, d.Timing.Phase)).ToList();
            var winner = LoopFirstRanking.Pick(groups, g => g.Key.Cycle == NifControllerExtractor.CycleLoop, OffsetTiesCounter);
            if (winner == null)
                return null;

            NifAnimationStats.Add(OffsetDroppedStandaloneCounter, drivers.Count - winner.Count());
            var tracks = winner.Select(d => d.Track).GroupBy(t => t.NodeName, StringComparer.Ordinal).Select(g => g.First()).ToList();
            return new SequenceClip(
                StandaloneClipName,
                winner.Min(d => d.Timing.Start),
                winner.Max(d => d.Timing.Stop),
                winner.Key.Cycle,
                Array.Empty<TextKeyEvent>(),
                tracks,
                Frequency: winner.Key.Frequency,
                Phase: winner.Key.Phase);
        }

        // Light's model-space matrix: its own local transform, then each ancestor's.
        private static Matrix4x4 ChainMatrix(IReadOnlyList<ChainNode> chain, Func<string?, NodeTrack?> trackOf, float time)
        {
            var world = Matrix4x4.Identity;
            foreach (var node in chain)
                world *= PoseAt(node, trackOf(node.Name), time).ToMatrix();
            return world;
        }

        private static LocalPose PoseAt(ChainNode node, NodeTrack? track, float time)
        {
            if (track == null)
                return node.Rest;
            return new LocalPose(
                Evaluate(track.Translation, time, node.Rest.Translation, Vector3.Lerp),
                Evaluate(track.Rotation, time, node.Rest.Rotation, Quaternion.Slerp),
                Evaluate(track.Scale, time, node.Rest.Scale, Vector3.Lerp));
        }

        // Linear between keys, held outside them; a channel without keys keeps the rest value.
        private static T Evaluate<T>(IReadOnlyList<Sample<T>> samples, float time, T rest, Func<T, T, float, T> lerp)
        {
            if (samples.Count == 0)
                return rest;
            if (time <= samples[0].Time)
                return samples[0].Value;
            for (int i = 1; i < samples.Count; i++)
            {
                if (time > samples[i].Time)
                    continue;
                float span = samples[i].Time - samples[i - 1].Time;
                return span <= 0f ? samples[i].Value : lerp(samples[i - 1].Value, samples[i].Value, (time - samples[i - 1].Time) / span);
            }
            return samples[^1].Value;
        }

        private static List<float> SampleTimes(float start, float stop)
        {
            var times = new List<float>();
            int steps = (int)MathF.Ceiling((stop - start) * NifKeySampler.SampleRate);
            for (int i = 0; i < steps; i++)
                times.Add(start + i / NifKeySampler.SampleRate);
            times.Add(stop);
            return times;
        }

        private static LocalPose RestPose(NiAVObject node)
        {
            var matrix = GameCoordinates.ToGltfTransform(node.Rotation, node.Translation, node.Scale);
            if (!Matrix4x4.Decompose(matrix, out var scale, out var rotation, out var translation))
                throw new NifAnimationException($"transform of '{node.Name?.String}' cannot be decomposed");
            return new LocalPose(translation, rotation, scale);
        }

        // Parent chain of a block, nearest first, without the root (parentless) node.
        private static IEnumerable<NiNode> Ancestors(NifFile nif, IReadOnlyDictionary<int, int> parentOf, int block)
        {
            var visited = new HashSet<int> { block };
            int current = block;
            while (parentOf.TryGetValue(current, out int parent))
            {
                if (!visited.Add(parent))
                    throw new NifAnimationException($"node hierarchy loops back to block {parent}");
                if (!parentOf.ContainsKey(parent))
                    yield break;
                yield return (NiNode)nif.Blocks[parent];
                current = parent;
            }
        }
    }
}
