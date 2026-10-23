using System.Collections.Concurrent;

namespace AssetConverter
{
    // Named counters for animation and skin handling. Harmless-but-unsupported
    // content (shader/visibility/particle controllers, odd cycle types) is counted
    // here and printed in the NIF conversion summary rather than ignored silently.
    internal static class NifAnimationStats
    {
        public const string FilesWithAnimation = "files with exported animation";
        public const string SequencesExported = "sequences exported";
        public const string SequencesPoseOnly = "pose-only sequences exported (single key per channel from the default transform)";
        public const string SequencesDroppedNoKeys = "sequences dropped (no keyed or posed transform channel)";
        public const string SequencesRenamedDuplicate = "sequences renamed (duplicate name in file)";
        public const string SequencesReverseCycleAsLoop = "sequences with reverse cycle exported as loop";
        public const string SequencesNonUnitFrequency = "sequences with frequency != 1 (frequency not exported in animations)";
        public const string ChannelsExported = "channels exported";
        public const string KeysExported = "keys exported";
        public const string TracksResampled = "tracks resampled at 30 Hz";
        public const string TracksConstant = "tracks collapsed to one key";
        public const string BlocksWithoutKeys = "controlled blocks without keys";
        public const string NodesRenamedForUniqueness = "glTF nodes renamed to keep names unique (animated or skinned files)";
        public const string QuadraticRotationTracksAsSlerp = "quadratic rotation tracks approximated as slerp (format stores no tangents)";
        public const string UnsupportedPrefix = "unsupported (not exported): ";
        public const string SkinsRigidified = "skinned shapes rigidified";
        public const string SkinsTrueSkinned = "skinned shapes exported as glTF skin";
        public const string SkinsFailed = "skinned shapes failed";
        public const string SkinStraddlingTriangles = "rigidified triangles spanning several bones (assigned to first vertex's bone)";

        private static readonly ConcurrentDictionary<string, long> _counters = new();

        public static void Reset() => _counters.Clear();

        public static void Add(string counter, long amount = 1)
            => _counters.AddOrUpdate(counter, amount, (_, current) => current + amount);

        public static void AddUnsupported(string what) => Add(UnsupportedPrefix + what);

        // Counters of the effect, particle, addon and light pipeline, printed in their own summary section.
        private static readonly string[] EffectCounterPrefixes =
        [
            "effect shapes", "refraction shapes", "particle", "light offset", "light block", "light controller", "shader/light",
            "second controller for target", "BSEffectShaderProperty", "BSLightingShaderProperty", "NiLightColorController",
            "NiTransformController on the light block", "shader property controller", "standalone transform controllers",
            ClipTiming.RangeReplacedCounter, ControllerSources.NonFiniteKeysCounter,
        ];

        private static bool IsEffectCounter(string counter)
        {
            string name = counter.StartsWith(UnsupportedPrefix, StringComparison.Ordinal) ? counter[UnsupportedPrefix.Length..] : counter;
            return EffectCounterPrefixes.Any(prefix => name.StartsWith(prefix, StringComparison.Ordinal));
        }

        public static IEnumerable<string> SummaryLines() => Lines(_ => true);

        public static IEnumerable<string> AnimationSummaryLines() => Lines(counter => !IsEffectCounter(counter));

        public static IEnumerable<string> EffectSummaryLines() => Lines(IsEffectCounter);

        private static IEnumerable<string> Lines(Func<string, bool> include)
            => _counters.Where(kv => include(kv.Key)).OrderBy(kv => kv.Key, StringComparer.Ordinal).Select(kv => $"{kv.Key}: {kv.Value}");
    }
}
