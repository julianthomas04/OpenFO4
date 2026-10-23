using NiflySharp.Blocks;

namespace AssetConverter
{
    /// <summary>Cycle type, time range, frequency and phase of one time controller or controller sequence.</summary>
    internal sealed record ClipTiming(string Cycle, float Start, float Stop, float Frequency, float Phase)
    {
        public const string RangeReplacedCounter = "controller time ranges replaced by the key range (empty, inverted, non-finite or not covering the keys)";

        private const float CoverageTolerance = 1e-4f;

        public static ClipTiming Of(NiTimeController controller)
            => new(NifControllerExtractor.CycleName(controller.Flags.CycleType), controller.StartTime, controller.StopTime, controller.Frequency, controller.Phase);

        public static ClipTiming Of(NiControllerSequence sequence)
            => new(NifControllerExtractor.CycleName(sequence.CycleType), sequence.StartTime, sequence.StopTime, sequence.Frequency, sequence.Phase);

        /// <summary>
        /// The timing with its range replaced by [<paramref name="firstKey"/>, <paramref name="lastKey"/>] when the stored range is
        /// non-finite, inverted, empty over several keys or does not cover the keys. Replacements are counted.
        /// </summary>
        public ClipTiming CoveringKeys(float firstKey, float lastKey)
        {
            bool usable = float.IsFinite(Start) && float.IsFinite(Stop) && Start <= Stop
                && Start <= firstKey + CoverageTolerance && Stop >= lastKey - CoverageTolerance;
            if (usable)
                return this;
            NifAnimationStats.Add(RangeReplacedCounter);
            return this with { Start = firstKey, Stop = lastKey };
        }
    }
}
