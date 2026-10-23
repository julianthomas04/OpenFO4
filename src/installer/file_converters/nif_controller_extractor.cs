using System.Diagnostics.CodeAnalysis;
using System.Numerics;
using NiflySharp;
using NiflySharp.Blocks;
using NiflySharp.Enums;
using NiflySharp.Structs;

namespace AssetConverter
{
    // Track values are already in glTF space (Y-up, Godot scale).
    internal sealed record NodeTrack(
        string NodeName,
        IReadOnlyList<Sample<Vector3>> Translation,
        IReadOnlyList<Sample<Quaternion>> Rotation,
        IReadOnlyList<Sample<Vector3>> Scale)
    {
        public bool HasKeys => Translation.Count > 0 || Rotation.Count > 0 || Scale.Count > 0;
    }

    internal sealed record TextKeyEvent(float Time, string Key);

    internal sealed record SequenceClip(
        string Name,
        float Start,
        float Stop,
        string Cycle,
        IReadOnlyList<TextKeyEvent> TextKeys,
        IReadOnlyList<NodeTrack> Tracks,
        bool PoseOnly = false,
        float Frequency = 1f,
        float Phase = 0f);

    // Reads NiControllerManager sequences into engine-neutral clips. Only transform
    // channels (NiTransformInterpolator) are exported; every other controller kind
    // is counted in NifAnimationStats so the summary shows what was left out.
    internal static class NifControllerExtractor
    {
        public const string CycleLoop = "loop";
        public const string CycleClamp = "clamp";

        public static IReadOnlyList<SequenceClip> Extract(NifFile nif, Action<string> reportWarning)
        {
            CountUnexportedControllers(nif);

            var clips = new List<SequenceClip>();
            var usedNames = new HashSet<string>(StringComparer.Ordinal);
            foreach (var manager in nif.Blocks.OfType<NiControllerManager>())
            {
                foreach (int sequenceIndex in manager.ControllerSequences.Indices ?? new List<int>())
                {
                    try
                    {
                        if (!TryGetBlock<NiControllerSequence>(nif, sequenceIndex, "controller sequence", out var sequence))
                            continue;
                        var clip = ExtractSequence(nif, sequence, usedNames);
                        if (clip != null)
                            clips.Add(clip);
                    }
                    catch (NifAnimationException ex)
                    {
                        reportWarning(ex.Message);
                    }
                }
            }
            return clips;
        }

        private sealed record KeylessBlock(string NodeName, NiQuatTransform Pose);

        private static SequenceClip? ExtractSequence(NifFile nif, NiControllerSequence sequence, HashSet<string> usedNames)
        {
            string? name = sequence.Name?.String;
            if (string.IsNullOrEmpty(name))
                throw new NifAnimationException("NiControllerSequence without a name");
            if (!float.IsFinite(sequence.StartTime) || !float.IsFinite(sequence.StopTime) || sequence.StopTime < sequence.StartTime)
                throw new NifAnimationException($"sequence '{name}': invalid time range {sequence.StartTime}..{sequence.StopTime}");

            var tracks = new List<NodeTrack>();
            var keyless = new List<KeylessBlock>();
            var targeted = new HashSet<string>(StringComparer.Ordinal);
            foreach (var block in sequence.ControlledBlocks ?? new List<ControlledBlock>())
            {
                var interpolator = FindTransformInterpolator(nif, block, name);
                if (interpolator == null)
                    continue;
                string? nodeName = block.NodeName?.String;
                if (string.IsNullOrEmpty(nodeName))
                    throw new NifAnimationException($"sequence '{name}': transform block without a node name");
                if (!targeted.Add(nodeName))
                    throw new NifAnimationException($"sequence '{name}': node '{nodeName}' is driven by more than one transform block");

                var data = KeyData(nif, interpolator, name, nodeName);
                var track = data == null ? null : BuildTrack(name, nodeName, data);
                if (track is { HasKeys: true })
                    tracks.Add(track);
                else
                    keyless.Add(new KeylessBlock(nodeName, interpolator.Transform));
            }

            bool poseOnly = tracks.Count == 0;
            if (poseOnly)
                tracks.AddRange(keyless.Select(PoseTrack).Where(t => t.HasKeys));
            else
                NifAnimationStats.Add(NifAnimationStats.BlocksWithoutKeys, keyless.Count);
            if (tracks.Count == 0)
            {
                NifAnimationStats.Add(NifAnimationStats.SequencesDroppedNoKeys);
                return null;
            }

            if (sequence.Frequency != 1f)
                NifAnimationStats.Add(NifAnimationStats.SequencesNonUnitFrequency);
            if (poseOnly)
                NifAnimationStats.Add(NifAnimationStats.SequencesPoseOnly);
            return new SequenceClip(
                UniqueName(name, usedNames),
                poseOnly ? 0f : sequence.StartTime,
                poseOnly ? 0f : sequence.StopTime,
                poseOnly ? CycleClamp : CycleName(sequence.CycleType),
                ReadTextKeys(nif, sequence),
                tracks,
                poseOnly,
                sequence.Frequency,
                sequence.Phase);
        }

        internal static bool TryGetBlock<T>(NifFile nif, int index, string what, [NotNullWhen(true)] out T? block) where T : class
        {
            block = null;
            if (index < 0)
                return false;
            if (index >= nif.Blocks.Count)
                throw new NifAnimationException($"{what} block reference {index} is outside the {nif.Blocks.Count} blocks of the file");
            block = nif.Blocks[index] as T;
            return block != null;
        }

        private static NiTransformInterpolator? FindTransformInterpolator(NifFile nif, ControlledBlock block, string sequenceName)
        {
            if (TryGetBlock<NiTransformInterpolator>(nif, block.Interpolator.Index, $"sequence '{sequenceName}' interpolator", out var transform))
                return transform;
            if (TryGetBlock<NiTimeController>(nif, block.Controller.Index, $"sequence '{sequenceName}' controller", out var driven) && HasDedicatedExtractor(nif, driven))
                return null;
            string? controller = block.ControllerType?.String;
            string interpolatorType = block.Interpolator.Index >= 0 ? nif.Blocks[block.Interpolator.Index].GetType().Name : "no interpolator";
            NifAnimationStats.AddUnsupported($"sequence channel {(string.IsNullOrEmpty(controller) ? "unnamed controller" : controller)} via {interpolatorType}");
            return null;
        }

        private static NiTransformData? KeyData(NifFile nif, NiTransformInterpolator interpolator, string sequenceName, string nodeName)
            => TryGetBlock<NiTransformData>(nif, interpolator.Data.Index, $"sequence '{sequenceName}' node '{nodeName}' key data", out var data) ? data : null;

        private static NodeTrack PoseTrack(KeylessBlock block)
        {
            var pose = block.Pose;
            var translation = new List<Sample<Vector3>>();
            var rotation = new List<Sample<Quaternion>>();
            var scale = new List<Sample<Vector3>>();
            var t = pose.Translation;
            if (ControllerSources.IsSet(t.X) && ControllerSources.IsSet(t.Y) && ControllerSources.IsSet(t.Z))
                translation.Add(new Sample<Vector3>(0f, GameCoordinates.ToGltfPosition(t)));
            var q = GameCoordinates.FromNiflySharp(pose.Rotation);
            if (ControllerSources.IsSet(q.X) && ControllerSources.IsSet(q.Y) && ControllerSources.IsSet(q.Z) && ControllerSources.IsSet(q.W) && q.LengthSquared() > 0f)
                rotation.Add(new Sample<Quaternion>(0f, GameCoordinates.ToGltfRotation(Quaternion.Normalize(q))));
            if (ControllerSources.IsSet(pose.Scale))
                scale.Add(new Sample<Vector3>(0f, new Vector3(pose.Scale)));
            return new NodeTrack(block.NodeName, translation, rotation, scale);
        }

        internal static NodeTrack BuildTrack(string sequenceName, string nodeName, NiTransformData data)
        {
            try
            {
                Validate(data);
                var translation = NifKeySampler.Vector(data.Translations.Interpolation, ToVectorKeys(data.Translations.Keys))
                    .Select(s => new Sample<Vector3>(s.Time, GameCoordinates.ToGltfPosition(s.Value)))
                    .ToList();
                var scale = NifKeySampler.Vector(data.Scales.Interpolation, ToVectorKeys(data.Scales.Keys))
                    .Select(s => new Sample<Vector3>(s.Time, new Vector3(s.Value.X)))
                    .ToList();
                return new NodeTrack(nodeName, translation, BuildRotation(data), scale);
            }
            catch (NifAnimationException ex)
            {
                throw new NifAnimationException($"sequence '{sequenceName}' node '{nodeName}': {ex.Message}", ex);
            }
        }

        private static void Validate(NiTransformData data)
        {
            RequireFinite(data.Translations.Keys, "translation", k => k.Time, k => k.Value.X + k.Value.Y + k.Value.Z);
            RequireFinite(data.Scales.Keys, "scale", k => k.Time, k => k.Value);
            if (data.NumRotationKeys == 0)
                return;
            if (data.RotationType == KeyType.XYZ_ROTATION_KEY)
            {
                if (data.XYZRotations == null || data.XYZRotations.Length != 3)
                    throw new NifAnimationException($"XYZ rotation needs 3 axis key groups, found {data.XYZRotations?.Length ?? 0}");
                foreach (var axis in data.XYZRotations)
                    RequireFinite(axis.Keys, "euler rotation", k => k.Time, k => k.Value);
            }
            else
            {
                if (data.QuaternionKeys == null || data.QuaternionKeys.Count != data.NumRotationKeys)
                    throw new NifAnimationException($"{data.NumRotationKeys} rotation keys declared but {data.QuaternionKeys?.Count ?? 0} present");
                RequireFinite(data.QuaternionKeys, "rotation", k => k.Time, k => k.Value.X + k.Value.Y + k.Value.Z + k.Value.W);
            }
        }

        private static void RequireFinite<T>(IEnumerable<T> keys, string what, Func<T, float> time, Func<T, float> value)
        {
            foreach (var key in keys ?? Enumerable.Empty<T>())
            {
                if (!float.IsFinite(time(key)) || !float.IsFinite(value(key)))
                    throw new NifAnimationException($"non-finite {what} key");
            }
        }

        private static IReadOnlyList<Sample<Quaternion>> BuildRotation(NiTransformData data)
        {
            if (data.NumRotationKeys == 0)
                return Array.Empty<Sample<Quaternion>>();
            var keys = data.RotationType == KeyType.XYZ_ROTATION_KEY
                ? NifKeySampler.EulerXyz(data.XYZRotations.Select(g => new AxisKeys(g.Interpolation, ToVectorKeys(g.Keys))).ToList())
                : NifKeySampler.Rotation(data.RotationType, (data.QuaternionKeys ?? new()).Select(k => new RotationKey(k.Time, GameCoordinates.FromNiflySharp(k.Value), k.TBC)).ToList());
            return keys.Select(s => new Sample<Quaternion>(s.Time, GameCoordinates.ToGltfRotation(s.Value))).ToList();
        }

        internal static List<VectorKey> ToVectorKeys(List<Key<Vector3>>? keys)
            => (keys ?? new()).Select(k => new VectorKey(k.Time, k.Value, k.Forward, k.Backward, k.TBC)).ToList();

        internal static List<VectorKey> ToVectorKeys(List<Key<float>>? keys)
            => (keys ?? new()).Select(k => new VectorKey(k.Time, new Vector3(k.Value), new Vector3(k.Forward), new Vector3(k.Backward), k.TBC)).ToList();

        private static IReadOnlyList<TextKeyEvent> ReadTextKeys(NifFile nif, NiControllerSequence sequence)
        {
            if (!TryGetBlock<NiTextKeyExtraData>(nif, sequence.TextKeys_NiBlockRef_NTKED.Index, $"sequence '{sequence.Name?.String}' text key", out var textKeys))
                return Array.Empty<TextKeyEvent>();
            return (textKeys.TextKeys ?? new())
                .Select(k => new TextKeyEvent(k.Time, k.Value?.String ?? ""))
                .ToList();
        }

        internal static string CycleName(CycleType cycle)
        {
            switch (cycle)
            {
                case CycleType.CYCLE_LOOP:
                    return CycleLoop;
                case CycleType.CYCLE_CLAMP:
                    return CycleClamp;
                default:
                    NifAnimationStats.Add(NifAnimationStats.SequencesReverseCycleAsLoop);
                    return CycleLoop;
            }
        }

        private static string UniqueName(string name, HashSet<string> usedNames)
        {
            if (usedNames.Add(name))
                return name;
            NifAnimationStats.Add(NifAnimationStats.SequencesRenamedDuplicate);
            for (int suffix = 2; ; suffix++)
            {
                string candidate = $"{name}#{suffix}";
                if (usedNames.Add(candidate))
                    return candidate;
            }
        }

        // Controllers of effect shaders, refraction shaders, lights and particle systems are exported by their own
        // extractors, which count what they leave out; counting them here too would report exported content as dropped.
        internal static bool HasDedicatedExtractor(NifFile nif, NiTimeController controller)
        {
            switch (controller)
            {
                case BSEffectShaderPropertyFloatController or BSEffectShaderPropertyColorController:
                case NiLightDimmerController or NiLightColorController:
                case NiPSysUpdateCtlr or NiPSysModifierCtlr:
                    return true;
                case BSLightingShaderPropertyFloatController or BSLightingShaderPropertyColorController:
                    return nif.Blocks.ElementAtOrDefault(controller.Target.Index) is BSLightingShaderProperty shader
                        && ((uint)shader.ShaderFlags_F4SPF1 & RefractionShaderData.RefractionFlag) != 0;
                default:
                    return false;
            }
        }

        private static void CountUnexportedControllers(NifFile nif)
        {
            foreach (var controller in nif.Blocks.OfType<NiTimeController>())
            {
                if (controller is NiControllerManager or NiMultiTargetTransformController or NiTransformController || HasDedicatedExtractor(nif, controller))
                    continue;
                NifAnimationStats.AddUnsupported($"controller block {controller.GetType().Name}");
            }
        }
    }
}
