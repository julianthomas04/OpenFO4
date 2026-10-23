using System.Numerics;
using System.Text.Json.Nodes;
using SharpGLTF.Schema2;

namespace AssetConverter
{
    // Writes extracted clips into a glTF document following the animation asset
    // contract: one animation per sequence, channels
    // targeting nodes by name, `anim` extras on every animated node.
    internal static class GltfAnimationWriter
    {
        public static void Write(ModelRoot model, IReadOnlyList<SequenceClip> clips, Action<string> reportWarning)
        {
            var nodesByName = model.LogicalNodes.GroupBy(n => n.Name ?? "").ToDictionary(g => g.Key, g => g.ToList());
            bool wroteAny = false;
            foreach (var clip in clips)
            {
                try
                {
                    var targets = ResolveTargets(clip, nodesByName);
                    if (targets.Count == 0)
                    {
                        NifAnimationStats.Add(NifAnimationStats.SequencesDroppedNoKeys);
                        continue;
                    }
                    WriteClip(model, clip, targets);
                    wroteAny = true;
                }
                catch (NifAnimationException ex)
                {
                    reportWarning(ex.Message);
                }
            }
            if (wroteAny)
                NifAnimationStats.Add(NifAnimationStats.FilesWithAnimation);
        }

        private static Dictionary<string, Node> ResolveTargets(SequenceClip clip, Dictionary<string, List<Node>> nodesByName)
        {
            var targets = new Dictionary<string, Node>(StringComparer.Ordinal);
            foreach (var track in clip.Tracks)
            {
                if (clip.PoseOnly && (!nodesByName.TryGetValue(track.NodeName, out var poseCandidates) || PickTarget(poseCandidates) == null))
                    continue;
                if (!nodesByName.TryGetValue(track.NodeName, out var candidates))
                    throw new NifAnimationException($"sequence '{clip.Name}': target node '{track.NodeName}' does not exist in the converted scene");
                targets[track.NodeName] = PickTarget(candidates)
                    ?? throw new NifAnimationException($"sequence '{clip.Name}': target node name '{track.NodeName}' is used by {candidates.Count} nodes and cannot be addressed");
            }
            return targets;
        }

        // A NiNode and the shape under it may share a name; the animation drives the
        // NiNode, which is the only candidate without a mesh.
        private static Node? PickTarget(List<Node> candidates)
        {
            if (candidates.Count == 1)
                return candidates[0];
            var bare = candidates.Where(n => n.Mesh == null).ToList();
            return bare.Count == 1 ? bare[0] : null;
        }

        private static void WriteClip(ModelRoot model, SequenceClip clip, Dictionary<string, Node> targets)
        {
            var animation = model.CreateAnimation(clip.Name);
            foreach (var track in clip.Tracks.Where(t => targets.ContainsKey(t.NodeName)))
            {
                var node = targets[track.NodeName];
                if (track.Translation.Count > 0)
                    animation.CreateTranslationChannel(node, ToKeyframes(track.Translation), true);
                if (track.Rotation.Count > 0)
                    animation.CreateRotationChannel(node, ToKeyframes(track.Rotation), true);
                if (track.Scale.Count > 0)
                    animation.CreateScaleChannel(node, ToKeyframes(track.Scale), true);
                MarkAnimated(node);

                int channels = (track.Translation.Count > 0 ? 1 : 0) + (track.Rotation.Count > 0 ? 1 : 0) + (track.Scale.Count > 0 ? 1 : 0);
                NifAnimationStats.Add(NifAnimationStats.ChannelsExported, channels);
                NifAnimationStats.Add(NifAnimationStats.KeysExported, track.Translation.Count + track.Rotation.Count + track.Scale.Count);
            }
            animation.Extras = ClipExtras(clip);
            NifAnimationStats.Add(NifAnimationStats.SequencesExported);
        }

        private static void MarkAnimated(Node node)
        {
            var extras = GltfNodeExtras.Existing(node.Extras);
            extras["anim"] = true;
            node.Extras = extras;
        }

        private static SortedDictionary<float, T> ToKeyframes<T>(IReadOnlyList<Sample<T>> samples)
        {
            var keyframes = new SortedDictionary<float, T>();
            foreach (var sample in samples)
                keyframes[sample.Time] = sample.Value;
            return keyframes;
        }

        private static JsonObject ClipExtras(SequenceClip clip)
        {
            var textKeys = new JsonArray();
            foreach (var key in clip.TextKeys)
                textKeys.Add(new JsonObject { ["t"] = key.Time, ["key"] = key.Key });
            return new JsonObject
            {
                ["start"] = clip.Start,
                ["stop"] = clip.Stop,
                ["cycle"] = clip.Cycle,
                ["text_keys"] = textKeys,
            };
        }
    }
}
