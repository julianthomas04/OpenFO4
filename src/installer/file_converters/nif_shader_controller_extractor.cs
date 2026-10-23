using NiflySharp;
using NiflySharp.Blocks;
using NiflySharp.Enums;

namespace AssetConverter
{
    // Values are scalars (one component) or colours / vectors (three components).
    internal sealed record ControllerTrack(
        string Target,
        string Cycle,
        float Start,
        float Stop,
        float Frequency,
        float Phase,
        IReadOnlyList<float> Times,
        IReadOnlyList<float[]> Values);

    internal sealed record ShaderControllerResult(IReadOnlyList<ControllerTrack> Tracks, IReadOnlyList<string> Unsupported)
    {
        public static ShaderControllerResult Empty { get; } = new(Array.Empty<ControllerTrack>(), Array.Empty<string>());
    }

    // Turns the time controllers hanging off a shader property or a light into engine-neutral
    // tracks. Key data is read from the controller's own interpolator or, for manager-controlled
    // (blend) interpolators, from the NiControllerSequence controlled block that references the
    // controller (see ControllerSources.Resolve). Non-linear keys are resampled at 30 Hz by NifKeySampler. Controllers outside the
    // supported variable set are counted in NifAnimationStats and returned, never dropped silently.
    internal static class ShaderControllerTracks
    {
        public const string EmissiveMultiple = "emissive_mult";
        public const string Alpha = "alpha";
        public const string UOffset = "u_offset";
        public const string VOffset = "v_offset";
        public const string UScale = "u_scale";
        public const string VScale = "v_scale";
        public const string EmissiveColor = "emissive_color";
        public const string Dimmer = "dimmer";
        public const string LightColorTarget = "light_color";
        public const string RefractionStrength = "refraction_strength";

        public const string SequenceTiesCounter = "shader/light controllers with several equally ranked sequences (first in file order used)";
        public const string WithoutKeysCounter = "shader/light controllers without key data";

        private static readonly Dictionary<EffectShaderControlledVariable, string> EffectFloatTargets =
            new()
            {
                [EffectShaderControlledVariable.EmissiveMultiple] = EmissiveMultiple,
                [EffectShaderControlledVariable.AlphaTransparency] = Alpha,
                [EffectShaderControlledVariable.UOffset] = UOffset,
                [EffectShaderControlledVariable.UScale] = UScale,
                [EffectShaderControlledVariable.VOffset] = VOffset,
                [EffectShaderControlledVariable.VScale] = VScale,
            };

        private static readonly Dictionary<LightingShaderControlledFloat, string> LightingFloatTargets =
            new()
            {
                [LightingShaderControlledFloat.RefractionStrength] = RefractionStrength,
                [LightingShaderControlledFloat.UOffset] = UOffset,
                [LightingShaderControlledFloat.UScale] = UScale,
                [LightingShaderControlledFloat.VOffset] = VOffset,
                [LightingShaderControlledFloat.VScale] = VScale,
            };

        private enum Kind { Scalar, Color }

        private readonly record struct Mapping(string Target, Kind Kind);

        private abstract record Classification;
        private sealed record Supported(Mapping Mapping) : Classification;
        private sealed record Unsupported(string Description) : Classification;

        public static ShaderControllerResult ForShader(NifFile nif, int shaderBlock)
        {
            if (!TryGetOwner(nif, shaderBlock, out var shader))
                throw new NifAnimationException($"shader block {shaderBlock} is not a block with controllers");
            return Collect(nif, shader, ClassifyShaderController);
        }

        public static ShaderControllerResult ForLight(NifFile nif, int lightBlock)
        {
            if (!TryGetOwner(nif, lightBlock, out var light) || light is not NiLight)
                throw new NifAnimationException($"light block {lightBlock} is not a NiLight");
            return Collect(nif, light, ClassifyLightController);
        }

        private static bool TryGetOwner(NifFile nif, int index, out NiObjectNET owner)
        {
            owner = null!;
            if (index < 0 || index >= nif.Blocks.Count || nif.Blocks[index] is not NiObjectNET found)
                return false;
            owner = found;
            return true;
        }

        private static Classification ClassifyShaderController(NiTimeController controller)
        {
            switch (controller)
            {
                case BSEffectShaderPropertyFloatController effectFloat:
                    return EffectFloatTargets.TryGetValue(effectFloat.ControlledVariable, out var floatTarget)
                        ? new Supported(new Mapping(floatTarget, Kind.Scalar))
                        : new Unsupported($"{nameof(BSEffectShaderPropertyFloatController)} variable {effectFloat.ControlledVariable}");
                case BSEffectShaderPropertyColorController effectColor:
                    return effectColor.ControlledColor == EffectShaderControlledColor.EmissiveColor
                        ? new Supported(new Mapping(EmissiveColor, Kind.Color))
                        : new Unsupported($"{nameof(BSEffectShaderPropertyColorController)} colour {effectColor.ControlledColor}");
                case BSLightingShaderPropertyFloatController lightingFloat:
                    return LightingFloatTargets.TryGetValue(lightingFloat.ControlledVariable, out var lightingTarget)
                        ? new Supported(new Mapping(lightingTarget, Kind.Scalar))
                        : new Unsupported($"{nameof(BSLightingShaderPropertyFloatController)} variable {lightingFloat.ControlledVariable}");
                case BSLightingShaderPropertyColorController lightingColor:
                    return new Unsupported($"{nameof(BSLightingShaderPropertyColorController)} colour {lightingColor.ControlledColor}");
                default:
                    return new Unsupported($"shader property controller {controller.GetType().Name}");
            }
        }

        private static Classification ClassifyLightController(NiTimeController controller)
        {
            switch (controller)
            {
                case NiLightDimmerController:
                    return new Supported(new Mapping(Dimmer, Kind.Scalar));
                case NiLightColorController { TargetColor: LightColor.LC_DIFFUSE }:
                    return new Supported(new Mapping(LightColorTarget, Kind.Color));
                case NiLightColorController colorController:
                    return new Unsupported($"{nameof(NiLightColorController)} target {colorController.TargetColor}");
                case NiTransformController:
                    return new Unsupported($"{nameof(NiTransformController)} on the light block");
                default:
                    return new Unsupported($"light controller {controller.GetType().Name}");
            }
        }

        private static ShaderControllerResult Collect(NifFile nif, NiObjectNET owner, Func<NiTimeController, Classification> classify)
        {
            var tracks = new List<ControllerTrack>();
            var unsupported = new List<string>();
            var targets = new HashSet<string>(StringComparer.Ordinal);
            foreach (var (index, controller) in ControllerChain(nif, owner))
            {
                switch (classify(controller))
                {
                    case Unsupported(var description):
                        unsupported.Add(description);
                        NifAnimationStats.AddUnsupported(description);
                        break;
                    case Supported(var mapping):
                        var track = BuildTrack(nif, index, controller, mapping);
                        if (track == null)
                            break;
                        if (targets.Add(track.Target))
                        {
                            tracks.Add(track);
                            break;
                        }
                        string duplicate = $"second controller for target {track.Target}";
                        unsupported.Add(duplicate);
                        NifAnimationStats.AddUnsupported(duplicate);
                        break;
                }
            }
            return new ShaderControllerResult(tracks, unsupported);
        }

        internal static IEnumerable<(int Index, NiTimeController Controller)> ControllerChain(NifFile nif, NiObjectNET owner)
        {
            var visited = new HashSet<int>();
            int index = owner.Controller.Index;
            while (index >= 0)
            {
                if (index >= nif.Blocks.Count)
                    throw new NifAnimationException($"controller block reference {index} is outside the {nif.Blocks.Count} blocks of the file");
                if (!visited.Add(index))
                    throw new NifAnimationException($"controller chain loops back to block {index}");
                if (nif.Blocks[index] is not NiTimeController controller)
                    throw new NifAnimationException($"block {index} in a controller chain is a {nif.Blocks[index].GetType().Name}, not a time controller");
                yield return (index, controller);
                index = controller.NextController.Index;
            }
        }

        private static ControllerTrack? BuildTrack(NifFile nif, int controllerIndex, NiTimeController controller, Mapping mapping)
        {
            if (controller is not NiSingleInterpController single)
                throw new NifAnimationException($"{controller.GetType().Name} at block {controllerIndex} carries no interpolator");

            Func<NifFile, int, KeyList?> read = mapping.Kind == Kind.Color ? ControllerSources.ReadColor : ControllerSources.ReadFloat;
            var resolved = ControllerSources.Resolve(nif, controllerIndex, controller, single.Interpolator.Index, read, SequenceTiesCounter);
            if (resolved == null)
            {
                NifAnimationStats.Add(WithoutKeysCounter);
                return null;
            }
            return Sample(mapping, resolved);
        }

        private static ControllerTrack Sample(Mapping mapping, ResolvedKeys resolved)
        {
            var timing = resolved.CoveredTiming();
            var keys = resolved.Keys;
            var times = keys.HasKeys ? keys.Times : new[] { timing.Start };
            var values = keys.HasKeys ? keys.Values : new[] { Components(keys.Constant!, mapping.Kind) };
            return new ControllerTrack(mapping.Target, timing.Cycle, timing.Start, timing.Stop, timing.Frequency, timing.Phase, times, values);
        }

        private static float[] Components(float[] constant, Kind kind) => kind == Kind.Color ? constant : new[] { constant[0] };
    }
}
