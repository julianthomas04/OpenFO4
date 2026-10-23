using System.Numerics;

namespace AssetConverter.Havok
{
    /// <summary>
    /// Decodes hkaLosslessCompressedAnimation. Every track component carries a u16 descriptor: the low two bits are the kind
    /// (0 default value, 1 static value, 2 dynamic channel), the rest is the index into the static or dynamic array.
    /// Dynamic arrays are frame-major: value of channel c at frame f sits at f * channelCount + c.
    /// </summary>
    internal static class LosslessAnimationDecoder
    {
        private const int VectorDescriptorStride = 4;
        private const int KindMask = 0x3;
        private const int IndexShift = 2;
        private const int DefaultKind = 0;
        private const int StaticKind = 1;
        private const int DynamicKind = 2;
        private const long MaxDecodedTransforms = 1L << 20;

        public static HavokTransform[][] Decode(HavokLosslessAnimation a)
        {
            if (a.FrameCount < 0 || a.TransformTrackCount < 0 || (long)a.FrameCount * a.TransformTrackCount > MaxDecodedTransforms)
                throw new HavokFormatException($"Lossless animation of {a.FrameCount} frames x {a.TransformTrackCount} tracks is out of range");

            RequireDescriptors(a.TranslationDescriptors.Count, a.TransformTrackCount * VectorDescriptorStride, "translation");
            RequireDescriptors(a.RotationDescriptors.Count, a.TransformTrackCount, "rotation");
            RequireDescriptors(a.ScaleDescriptors.Count, a.TransformTrackCount * VectorDescriptorStride, "scale");

            var translations = new ChannelStore<float>(a.DynamicTranslations, a.StaticTranslations, a.FrameCount, (long)a.TransformTrackCount * 3, "translation");
            var rotations = new ChannelStore<Quaternion>(a.DynamicRotations, a.StaticRotations, a.FrameCount, a.TransformTrackCount, "rotation");
            var scales = new ChannelStore<float>(a.DynamicScales, a.StaticScales, a.FrameCount, (long)a.TransformTrackCount * 3, "scale");

            var tracks = new HavokTransform[a.TransformTrackCount][];
            for (int t = 0; t < tracks.Length; t++)
            {
                var frames = new HavokTransform[a.FrameCount];
                ushort rotationDescriptor = a.RotationDescriptors[t];
                for (int f = 0; f < frames.Length; f++)
                {
                    frames[f] = new HavokTransform(
                        ReadVector(translations, a.TranslationDescriptors, t, f, 0f),
                        NormalizeRotation(rotations.Read(rotationDescriptor, f, Quaternion.Identity), t, f),
                        ReadVector(scales, a.ScaleDescriptors, t, f, 1f));
                }
                tracks[t] = frames;
            }
            return tracks;
        }

        private static void RequireDescriptors(int actual, long required, string what)
        {
            if (actual < required)
                throw new HavokFormatException($"{actual} {what} descriptors cannot cover the {required} the tracks need");
        }

        private static Quaternion NormalizeRotation(Quaternion rotation, int track, int frame)
        {
            float lengthSquared = rotation.LengthSquared();
            if (!(lengthSquared > 0f) || float.IsInfinity(lengthSquared))
                throw new HavokFormatException($"Rotation of track {track} at frame {frame} is zero or not finite and cannot be normalized");
            return Quaternion.Normalize(rotation);
        }

        private static Vector3 ReadVector(ChannelStore<float> store, IReadOnlyList<ushort> descriptors, int track, int frame, float fallback)
        {
            int first = track * VectorDescriptorStride;
            return new Vector3(
                store.Read(descriptors[first], frame, fallback),
                store.Read(descriptors[first + 1], frame, fallback),
                store.Read(descriptors[first + 2], frame, fallback));
        }

        private sealed class ChannelStore<T>
        {
            private readonly IReadOnlyList<T> _dynamic;
            private readonly IReadOnlyList<T> _static;
            private readonly int _dynamicChannels;
            private readonly string _what;

            public ChannelStore(IReadOnlyList<T> dynamicValues, IReadOnlyList<T> staticValues, int frameCount, long maxChannels, string what)
            {
                if (frameCount == 0 ? dynamicValues.Count != 0 : dynamicValues.Count % frameCount != 0)
                    throw new HavokFormatException($"{dynamicValues.Count} dynamic {what} values do not divide into {frameCount} frames");
                if (dynamicValues.Count > frameCount * maxChannels)
                    throw new HavokFormatException($"{dynamicValues.Count} dynamic {what} values exceed {frameCount} frames x {maxChannels} channels");
                _dynamic = dynamicValues;
                _static = staticValues;
                _dynamicChannels = frameCount == 0 ? 0 : dynamicValues.Count / frameCount;
                _what = what;
            }

            public T Read(ushort descriptor, int frame, T fallback)
            {
                int index = descriptor >> IndexShift;
                switch (descriptor & KindMask)
                {
                    case DefaultKind:
                        if (index != 0) throw new HavokFormatException($"Default {_what} descriptor {descriptor} carries index {index}");
                        return fallback;
                    case StaticKind:
                        if (index >= _static.Count) throw new HavokFormatException($"Static {_what} index {index} outside {_static.Count} values");
                        return _static[index];
                    case DynamicKind:
                        if (index >= _dynamicChannels) throw new HavokFormatException($"Dynamic {_what} channel {index} outside {_dynamicChannels} channels");
                        return _dynamic[frame * _dynamicChannels + index];
                    default:
                        throw new HavokFormatException($"Unknown {_what} descriptor kind {descriptor & KindMask}");
                }
            }
        }
    }
}
