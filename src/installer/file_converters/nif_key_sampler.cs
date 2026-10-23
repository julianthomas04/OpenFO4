using System.Numerics;
using NiflySharp.Enums;
using NiflySharp.Structs;

namespace AssetConverter
{
    internal readonly record struct Sample<T>(float Time, T Value);

    internal readonly record struct VectorKey(float Time, Vector3 Value, Vector3 Forward, Vector3 Backward, TBC Tbc);

    internal readonly record struct RotationKey(float Time, Quaternion Value, TBC Tbc);

    internal readonly record struct AxisKeys(KeyType Type, IReadOnlyList<VectorKey> Keys);

    // Turns Gamebryo key groups into glTF-ready samples. LINEAR keys pass through
    // unchanged; every other interpolation is evaluated and resampled at
    // SampleRate so the engine only ever sees linear/slerp tracks.
    //
    //   QUADRATIC  cubic Hermite per segment. Forward of the left key and Backward
    //              of the right key are the Hermite tangents, used as stored: they
    //              are already expressed per segment (verified against vanilla data
    //              where |tangent| is of the order of the segment delta, not delta/dt).
    //   QUADRATIC rotation: nif.xml stores no tangents for quaternion keys (only a value
    //              per key), so there is nothing to build a squad from; the keys are slerped
    //              and the track is counted in NifAnimationStats.
    //   TBC        Kochanek-Bartels tangents with the 3ds-Max style 2*dt/(dtPrev+dtNext)
    //              spacing correction. Quaternion TBC runs the same tangents in log
    //              space and evaluates the segment as a squad (de Casteljau of slerps).
    //   CONST      step: the left key's value holds until the next key.
    //   XYZ        three scalar groups, composed as qz * qy * qx (R = Rz*Ry*Rx, X applied
    //              first). Verified against vanilla nodes whose rest rotation equals the
    //              first Euler key; the reverse order does not reproduce them.
    internal static class NifKeySampler
    {
        public const float SampleRate = 30f;
        private const float TimeEpsilon = 1e-5f;
        private const float ConstantTolerance = 1e-5f;
        private const float ConstantQuaternionTolerance = 1e-9f;

        public static IReadOnlyList<Sample<Vector3>> Vector(KeyType type, IReadOnlyList<VectorKey> keys)
        {
            keys = SortedUnique(keys, k => k.Time);
            if (keys.Count == 0)
                return Array.Empty<Sample<Vector3>>();
            if (IsLinear(type, keys.Count))
                return CollapseConstant(keys.Select(k => new Sample<Vector3>(k.Time, k.Value)).ToList(), VectorsEqual);
            var evaluate = VectorEvaluator(type, keys);
            return CollapseConstant(Resample(keys[0].Time, keys[^1].Time, evaluate), VectorsEqual);
        }

        public static IReadOnlyList<Sample<Quaternion>> Rotation(KeyType type, IReadOnlyList<RotationKey> keys)
        {
            keys = SortedUnique(keys, k => k.Time);
            if (keys.Count == 0)
                return Array.Empty<Sample<Quaternion>>();
            if (type == KeyType.QUADRATIC_KEY)
                NifAnimationStats.Add(NifAnimationStats.QuadraticRotationTracksAsSlerp);
            if (type == KeyType.QUADRATIC_KEY || IsLinear(type, keys.Count))
                return CollapseConstant(ContinuousHemisphere(keys.Select(k => new Sample<Quaternion>(k.Time, Quaternion.Normalize(k.Value)))), QuaternionsEqual);
            var evaluate = RotationEvaluator(type, keys);
            return CollapseConstant(ContinuousHemisphere(Resample(keys[0].Time, keys[^1].Time, evaluate)), QuaternionsEqual);
        }

        public static IReadOnlyList<Sample<Quaternion>> EulerXyz(IReadOnlyList<AxisKeys> axes)
        {
            if (axes.Count != 3)
                throw new NifAnimationException($"XYZ rotation needs 3 axis key groups, found {axes.Count}");
            var sortedAxes = axes.Select(a => new AxisKeys(a.Type, SortedUnique(a.Keys, k => k.Time))).ToList();
            var populated = sortedAxes.Where(a => a.Keys.Count > 0).ToList();
            if (populated.Count == 0)
                return Array.Empty<Sample<Quaternion>>();

            var angle = sortedAxes
                .Select(a => a.Keys.Count == 0 ? (_ => 0f) : ScalarEvaluator(a.Type, a.Keys))
                .ToArray();
            float first = populated.Min(a => a.Keys[0].Time);
            float last = populated.Max(a => a.Keys[^1].Time);
            Quaternion Evaluate(float t) =>
                Quaternion.CreateFromAxisAngle(Vector3.UnitZ, angle[2](t))
                * Quaternion.CreateFromAxisAngle(Vector3.UnitY, angle[1](t))
                * Quaternion.CreateFromAxisAngle(Vector3.UnitX, angle[0](t));
            return CollapseConstant(ContinuousHemisphere(Resample(first, last, Evaluate)), QuaternionsEqual);
        }

        // TBC tangents of a two-key track both equal the segment delta, which makes the
        // Hermite/squad segment exactly linear/slerp.
        private static bool IsLinear(KeyType type, int keyCount)
            => type == KeyType.LINEAR_KEY || (type == KeyType.TBC_KEY && keyCount == 2);

        private static Func<float, float> ScalarEvaluator(KeyType type, IReadOnlyList<VectorKey> keys)
        {
            var evaluate = VectorEvaluator(type, keys);
            return t => evaluate(t).X;
        }

        private static Func<float, Vector3> VectorEvaluator(KeyType type, IReadOnlyList<VectorKey> keys)
        {
            var times = keys.Select(k => k.Time).ToArray();
            switch (type)
            {
                case KeyType.LINEAR_KEY:
                    return t =>
                    {
                        var (i, u) = Locate(times, t);
                        return keys.Count == 1 ? keys[0].Value : Vector3.Lerp(keys[i].Value, keys[i + 1].Value, u);
                    };
                case KeyType.CONST_KEY:
                    return t => keys[HeldKeyIndex(times, t)].Value;
                case KeyType.QUADRATIC_KEY:
                    return t =>
                    {
                        var (i, u) = Locate(times, t);
                        return keys.Count == 1
                            ? keys[0].Value
                            : Hermite(keys[i].Value, keys[i].Forward, keys[i + 1].Backward, keys[i + 1].Value, u);
                    };
                case KeyType.TBC_KEY:
                    {
                        var deltas = SegmentDeltas(keys.Select(k => k.Value).ToArray(), (a, b) => b - a);
                        var (outgoing, incoming) = TcbTangents(times, keys.Select(k => k.Tbc).ToArray(), deltas);
                        return t =>
                        {
                            var (i, u) = Locate(times, t);
                            return keys.Count == 1
                                ? keys[0].Value
                                : Hermite(keys[i].Value, outgoing[i], incoming[i + 1], keys[i + 1].Value, u);
                        };
                    }
                default:
                    throw new NifAnimationException($"unsupported {type} key type on a vector track");
            }
        }

        private static Func<float, Quaternion> RotationEvaluator(KeyType type, IReadOnlyList<RotationKey> keys)
        {
            var times = keys.Select(k => k.Time).ToArray();
            var values = ContinuousHemisphere(keys.Select(k => new Sample<Quaternion>(k.Time, Quaternion.Normalize(k.Value))))
                .Select(s => s.Value).ToArray();
            switch (type)
            {
                case KeyType.CONST_KEY:
                    return t => values[HeldKeyIndex(times, t)];
                case KeyType.TBC_KEY:
                    {
                        var deltas = SegmentDeltas(values, (a, b) => Log(Quaternion.Conjugate(a) * b));
                        var (outgoing, incoming) = TcbTangents(times, keys.Select(k => k.Tbc).ToArray(), deltas);
                        return t =>
                        {
                            var (i, u) = Locate(times, t);
                            if (values.Length == 1) return values[0];
                            var a = values[i];
                            var b = values[i + 1];
                            var controlA = a * Exp(outgoing[i] / 3f);
                            var controlB = b * Exp(-incoming[i + 1] / 3f);
                            return Squad(a, controlA, controlB, b, u);
                        };
                    }
                default:
                    throw new NifAnimationException($"unsupported {type} key type on a rotation track");
            }
        }

        private static List<Sample<T>> Resample<T>(float first, float last, Func<float, T> evaluate)
        {
            NifAnimationStats.Add(NifAnimationStats.TracksResampled);
            var samples = new List<Sample<T>> { new(first, evaluate(first)) };
            if (last - first <= TimeEpsilon)
                return samples;
            for (int step = 1; ; step++)
            {
                float t = first + step / SampleRate;
                if (t >= last - TimeEpsilon)
                    break;
                samples.Add(new Sample<T>(t, evaluate(t)));
            }
            samples.Add(new Sample<T>(last, evaluate(last)));
            return samples;
        }

        private static (int Segment, float U) Locate(float[] times, float t)
        {
            if (times.Length == 1 || t <= times[0])
                return (0, 0f);
            if (t >= times[^1])
                return (times.Length - 2, 1f);
            int index = Array.BinarySearch(times, t);
            int segment = index >= 0 ? Math.Min(index, times.Length - 2) : ~index - 1;
            float span = times[segment + 1] - times[segment];
            return (segment, span <= 0f ? 0f : (t - times[segment]) / span);
        }

        // CONST: the left key holds until the next key; the last key holds from its own time on.
        private static int HeldKeyIndex(float[] times, float t)
            => t >= times[^1] ? times.Length - 1 : Locate(times, t).Segment;

        private static Vector3 Hermite(Vector3 p0, Vector3 m0, Vector3 m1, Vector3 p1, float u)
        {
            float u2 = u * u, u3 = u2 * u;
            return p0 * (2 * u3 - 3 * u2 + 1) + m0 * (u3 - 2 * u2 + u) + p1 * (-2 * u3 + 3 * u2) + m1 * (u3 - u2);
        }

        private static Vector3[] SegmentDeltas<T>(T[] values, Func<T, T, Vector3> delta)
        {
            var deltas = new Vector3[Math.Max(values.Length - 1, 0)];
            for (int i = 0; i < deltas.Length; i++)
                deltas[i] = delta(values[i], values[i + 1]);
            return deltas;
        }

        private static (Vector3[] Outgoing, Vector3[] Incoming) TcbTangents(float[] times, TBC[] tbc, Vector3[] deltas)
        {
            int n = times.Length;
            var outgoing = new Vector3[n];
            var incoming = new Vector3[n];
            if (n < 2)
                return (outgoing, incoming);
            outgoing[0] = deltas[0];
            incoming[n - 1] = deltas[n - 2];
            for (int i = 1; i < n - 1; i++)
            {
                float t = tbc[i].T, b = tbc[i].B, c = tbc[i].C;
                float dtPrev = times[i] - times[i - 1];
                float dtNext = times[i + 1] - times[i];
                float spacing = dtPrev + dtNext;
                float adjustOut = spacing <= 0f ? 1f : 2f * dtNext / spacing;
                float adjustIn = spacing <= 0f ? 1f : 2f * dtPrev / spacing;
                outgoing[i] = adjustOut * (0.5f * (1 - t) * (1 + b) * (1 + c) * deltas[i - 1]
                                           + 0.5f * (1 - t) * (1 - b) * (1 - c) * deltas[i]);
                incoming[i] = adjustIn * (0.5f * (1 - t) * (1 + b) * (1 - c) * deltas[i - 1]
                                          + 0.5f * (1 - t) * (1 - b) * (1 + c) * deltas[i]);
            }
            return (outgoing, incoming);
        }

        private static Quaternion Squad(Quaternion a, Quaternion controlA, Quaternion controlB, Quaternion b, float u)
        {
            var ab = Quaternion.Slerp(a, controlA, u);
            var mid = Quaternion.Slerp(controlA, controlB, u);
            var cb = Quaternion.Slerp(controlB, b, u);
            return Quaternion.Normalize(Quaternion.Slerp(Quaternion.Slerp(ab, mid, u), Quaternion.Slerp(mid, cb, u), u));
        }

        private static Vector3 Log(Quaternion unit)
        {
            float w = Math.Clamp(unit.W, -1f, 1f);
            float halfAngle = MathF.Acos(w);
            float s = MathF.Sin(halfAngle);
            if (s < 1e-6f)
                return Vector3.Zero;
            return new Vector3(unit.X, unit.Y, unit.Z) * (halfAngle / s);
        }

        private static Quaternion Exp(Vector3 v)
        {
            float halfAngle = v.Length();
            if (halfAngle < 1e-6f)
                return Quaternion.Identity;
            var axis = v / halfAngle;
            float s = MathF.Sin(halfAngle);
            return new Quaternion(axis.X * s, axis.Y * s, axis.Z * s, MathF.Cos(halfAngle));
        }

        private static List<T> SortedUnique<T>(IReadOnlyList<T> keys, Func<T, float> time)
        {
            var sorted = keys.Select((key, index) => (key, index)).OrderBy(p => time(p.key)).ThenBy(p => p.index).Select(p => p.key).ToList();
            var unique = new List<T>(sorted.Count);
            foreach (var key in sorted)
            {
                if (unique.Count > 0 && time(unique[^1]) == time(key))
                    unique[^1] = key;
                else
                    unique.Add(key);
            }
            return unique;
        }

        private static List<Sample<Quaternion>> ContinuousHemisphere(IEnumerable<Sample<Quaternion>> samples)
        {
            var result = new List<Sample<Quaternion>>();
            foreach (var sample in samples)
            {
                var q = sample.Value;
                if (result.Count > 0 && Quaternion.Dot(result[^1].Value, q) < 0f)
                    q = Quaternion.Negate(q);
                result.Add(new Sample<Quaternion>(sample.Time, q));
            }
            return result;
        }

        private static IReadOnlyList<Sample<T>> CollapseConstant<T>(IReadOnlyList<Sample<T>> samples, Func<T, T, bool> equal)
        {
            if (samples.Count > 1 && samples.All(s => equal(s.Value, samples[0].Value)))
            {
                NifAnimationStats.Add(NifAnimationStats.TracksConstant);
                return new[] { samples[0] };
            }
            return samples;
        }

        private static bool VectorsEqual(Vector3 a, Vector3 b) => (a - b).LengthSquared() <= ConstantTolerance * ConstantTolerance;

        private static bool QuaternionsEqual(Quaternion a, Quaternion b) => 1f - MathF.Abs(Quaternion.Dot(a, b)) <= ConstantQuaternionTolerance;
    }
}
