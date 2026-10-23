using System.Numerics;
using NiflySharp;
using NiflySharp.Blocks;
using NiflySharp.Enums;

namespace AssetConverter
{
    /// <summary>Sampled key data of one controller value: keys, or one constant when the interpolator carries no key groups.</summary>
    internal sealed record KeyList(IReadOnlyList<float> Times, IReadOnlyList<float[]> Values, float[]? Constant)
    {
        public bool HasKeys => Times.Count > 0;

        public static KeyList Fixed(params float[] constant) => new(Array.Empty<float>(), Array.Empty<float[]>(), constant);
    }

    /// <summary>Key data together with the timing it plays under; the timing's range is not yet checked against the keys.</summary>
    internal sealed record ResolvedKeys(KeyList Keys, ClipTiming Timing)
    {
        /// <summary>First and last key time; a constant sits at the range start (0 when that is not a finite number).</summary>
        public (float First, float Last) KeyRange
        {
            get
            {
                if (Keys.HasKeys)
                    return (Keys.Times[0], Keys.Times[^1]);
                float time = float.IsFinite(Timing.Start) ? Timing.Start : 0f;
                return (time, time);
            }
        }

        public ClipTiming CoveredTiming() => Timing.CoveringKeys(KeyRange.First, KeyRange.Last);
    }

    internal sealed record SequencePick<T>(T Source, ClipTiming Timing);

    /// <summary>Reads the key data behind a controller interpolator (float, point or bool), resampled to linear keys.</summary>
    internal static class ControllerSources
    {
        public const string NonFiniteKeysCounter = "controllers with non-finite key data (controller ignored)";

        private const float UnsetBound = 1e30f;

        /// <summary>An interpolator or NiQuatTransform value holds an "unset" sentinel of about -3.4e38 when it does not drive the channel.</summary>
        public static bool IsSet(float value) => float.IsFinite(value) && MathF.Abs(value) < UnsetBound;

        public static KeyList? ReadFloat(NifFile nif, int interpolatorIndex)
        {
            if (!NifControllerExtractor.TryGetBlock<NiFloatInterpolator>(nif, interpolatorIndex, "controller interpolator", out var interpolator))
                return null;
            if (NifControllerExtractor.TryGetBlock<NiFloatData>(nif, interpolator.Data.Index, "float key data", out var data) && data.Data.Keys is { Count: > 0 } keys)
            {
                if (keys.Any(key => !float.IsFinite(key.Time) || !float.IsFinite(key.Value)))
                    return NonFinite();
                return Resample(data.Data.Interpolation, NifControllerExtractor.ToVectorKeys(keys), v => new[] { v.X });
            }
            return IsSet(interpolator.Value) ? KeyList.Fixed(interpolator.Value) : null;
        }

        public static KeyList? ReadColor(NifFile nif, int interpolatorIndex)
        {
            if (!NifControllerExtractor.TryGetBlock<NiPoint3Interpolator>(nif, interpolatorIndex, "controller interpolator", out var interpolator))
                return null;
            if (NifControllerExtractor.TryGetBlock<NiPosData>(nif, interpolator.Data.Index, "position key data", out var data) && data.Data.Keys is { Count: > 0 } keys)
            {
                if (keys.Any(key => !float.IsFinite(key.Time) || !float.IsFinite(key.Value.X + key.Value.Y + key.Value.Z)))
                    return NonFinite();
                return Resample(data.Data.Interpolation, NifControllerExtractor.ToVectorKeys(keys), v => new[] { v.X, v.Y, v.Z });
            }
            var value = interpolator.Value;
            return IsSet(value.X) && IsSet(value.Y) && IsSet(value.Z) ? KeyList.Fixed(value.X, value.Y, value.Z) : null;
        }

        public static KeyList? ReadBool(NifFile nif, int interpolatorIndex)
        {
            if (!NifControllerExtractor.TryGetBlock<NiBoolInterpolator>(nif, interpolatorIndex, "controller interpolator", out var interpolator))
                return null;
            if (NifControllerExtractor.TryGetBlock<NiBoolData>(nif, interpolator.Data.Index, "bool key data", out var data) && data.Data.Keys is { Count: > 0 } keys)
            {
                if (keys.Any(key => !float.IsFinite(key.Time)))
                    return NonFinite();
                return new KeyList(keys.Select(k => k.Time).ToList(), keys.Select(k => new[] { k.Value == 0 ? 0f : 1f }).ToList(), null);
            }
            return interpolator.Value.HasValue ? KeyList.Fixed(interpolator.Value.Value ? 1f : 0f) : null;
        }

        /// <summary>
        /// Key data of one controller value: its own interpolator's keys, else the keys of the controller-sequence block that references
        /// the controller (<see cref="LoopFirstRanking"/> among several), else the interpolator's constant; null when there is nothing.
        /// </summary>
        public static ResolvedKeys? Resolve(NifFile nif, int controllerIndex, NiTimeController controller, int interpolatorIndex, Func<NifFile, int, KeyList?> read, string tiesCounter)
        {
            var own = read(nif, interpolatorIndex);
            var ownTiming = ClipTiming.Of(controller);
            if (own is { HasKeys: true })
                return new ResolvedKeys(own, ownTiming);

            var winner = PickFromSequences(nif, controllerIndex, index => read(nif, index), tiesCounter);
            if (winner != null)
                return new ResolvedKeys(winner.Source, winner.Timing);
            return own == null ? null : new ResolvedKeys(own, ownTiming);
        }

        public static SequencePick<T>? PickFromSequences<T>(NifFile nif, int controllerIndex, Func<int, T?> read, string tiesCounter) where T : class
        {
            var candidates = new List<SequencePick<T>>();
            foreach (var sequence in nif.Blocks.OfType<NiControllerSequence>())
            {
                foreach (var block in sequence.ControlledBlocks ?? new List<NiflySharp.Structs.ControlledBlock>())
                {
                    if (block.Controller.Index != controllerIndex)
                        continue;
                    var source = read(block.Interpolator.Index);
                    if (source != null)
                        candidates.Add(new SequencePick<T>(source, ClipTiming.Of(sequence)));
                }
            }
            return LoopFirstRanking.Pick(candidates, c => c.Timing.Cycle == NifControllerExtractor.CycleLoop, tiesCounter);
        }

        private static KeyList? NonFinite()
        {
            NifAnimationStats.Add(NonFiniteKeysCounter);
            return null;
        }

        private static KeyList Resample(KeyType interpolation, List<VectorKey> keys, Func<Vector3, float[]> project)
        {
            var samples = NifKeySampler.Vector(interpolation, keys);
            return new KeyList(samples.Select(s => s.Time).ToList(), samples.Select(s => project(s.Value)).ToList(), null);
        }
    }
}
