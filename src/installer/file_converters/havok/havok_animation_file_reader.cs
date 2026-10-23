using System.Diagnostics.CodeAnalysis;
using System.Numerics;

namespace AssetConverter.Havok
{
    /// <summary>Skeletons and animation bindings found under a packfile's hkRootLevelContainer.</summary>
    public sealed class HavokAnimationFile
    {
        public required IReadOnlyList<HavokSkeleton> Skeletons { get; init; }
        public required IReadOnlyList<HavokAnimationBinding> Bindings { get; init; }

        public bool HasSkeletons => Skeletons.Count > 0;
        public bool HasAnimations => Bindings.Count > 0;
    }

    /// <summary>
    /// Reads hkRootLevelContainer, hkaAnimationContainer, hkaSkeleton, hkaAnimationBinding and the animation
    /// classes from a hk_2014.1.0-r1 64-bit packfile.
    /// </summary>
    public static class HavokAnimationFileReader
    {
        private const string RootLevelContainerClass = "hkRootLevelContainer";
        private const string AnimationContainerClass = "hkaAnimationContainer";
        private const string SkeletonClass = "hkaSkeleton";
        private const string BindingClass = "hkaAnimationBinding";
        private const string SplineAnimationClass = "hkaSplineCompressedAnimation";
        private const string InterleavedAnimationClass = "hkaInterleavedUncompressedAnimation";
        private const string LosslessAnimationClass = "hkaLosslessCompressedAnimation";

        private const int NamedVariantSize = 0x18;
        private const int QsTransformSize = 48;
        private const int BoneSize = 16;
        private const int AnnotationTrackSize = 0x18;
        private const int AnnotationSize = 16;
        private const int QuaternionSize = 16;
        private const int LosslessVectorDescriptorShorts = 4;
        private const int PointerSize = 8;
        private const int MaxFrames = 1 << 20;
        private const int MaxTracks = 1 << 14;

        /// <summary>Reads a packfile that must hold an hkaAnimationContainer; use <see cref="TryRead"/> for arbitrary Havok files.</summary>
        public static HavokAnimationFile Read(HavokPackfile pack)
        {
            var container = FindAnimationContainer(pack)
                ?? throw new HavokFormatException($"{RootLevelContainerClass} holds no {AnimationContainerClass}");
            return ReadContainer(pack, container);
        }

        /// <summary>False for packfiles without an animation container (behavior graphs, character data, projects).</summary>
        public static bool TryRead(HavokPackfile pack, [NotNullWhen(true)] out HavokAnimationFile? file)
        {
            var container = FindAnimationContainer(pack);
            file = container is null ? null : ReadContainer(pack, container);
            return file is not null;
        }

        private static HavokAnimationFile ReadContainer(HavokPackfile pack, HavokObject container)
        {
            return new HavokAnimationFile
            {
                Skeletons = ReadObjectArray(pack, container.Offset + 0x10, SkeletonClass, "skeletons", ReadSkeleton),
                Bindings = ReadBindings(pack, container),
            };
        }

        private static HavokObject? FindAnimationContainer(HavokPackfile pack)
        {
            var root = pack.ObjectsOfClass(RootLevelContainerClass).FirstOrDefault();
            if (root is null) return null;

            var variants = pack.ReadArray(root.Offset, NamedVariantSize, "namedVariants");
            for (int i = 0; i < variants.Count; i++)
            {
                int variant = variants.DataOffset + i * NamedVariantSize;
                if (pack.ReadString(variant + 0x08) == AnimationContainerClass)
                    return pack.RequireObjectPointer(variant + 0x10, AnimationContainerClass, "root variant");
            }
            return null;
        }

        private static List<T> ReadObjectArray<T>(HavokPackfile pack, int arrayField, string expectedClass, string what,
            Func<HavokPackfile, HavokObject, T> read)
        {
            var array = pack.ReadArray(arrayField, PointerSize, what);
            var items = new List<T>(array.Count);
            for (int i = 0; i < array.Count; i++)
            {
                var obj = pack.RequireObjectPointer(array.DataOffset + i * PointerSize, expectedClass, what);
                items.Add(read(pack, obj));
            }
            return items;
        }

        private static List<HavokAnimationBinding> ReadBindings(HavokPackfile pack, HavokObject container)
        {
            var animationCache = new Dictionary<int, HavokAnimation>();
            return ReadObjectArray(pack, container.Offset + 0x30, BindingClass, "bindings",
                (p, binding) => ReadBinding(p, binding, animationCache));
        }

        private static HavokSkeleton ReadSkeleton(HavokPackfile pack, HavokObject obj)
        {
            int o = obj.Offset;
            var parents = pack.ReadArray(o + 0x18, 2, "skeleton parentIndices");
            var bones = pack.ReadArray(o + 0x28, BoneSize, "skeleton bones");
            var pose = pack.ReadArray(o + 0x38, QsTransformSize, "skeleton referencePose");
            if (parents.Count != bones.Count || pose.Count != bones.Count)
                throw new HavokFormatException($"Skeleton counts disagree: {parents.Count} parents, {bones.Count} bones, {pose.Count} pose entries");

            var parentIndices = new short[bones.Count];
            for (int i = 0; i < bones.Count; i++)
            {
                short parent = pack.Bytes.I16(parents.DataOffset + i * 2);
                if (parent < -1 || parent >= bones.Count)
                    throw new HavokFormatException($"Bone {i} has parent index {parent} outside [-1, {bones.Count})");
                parentIndices[i] = parent;
            }

            var names = new string[bones.Count];
            for (int i = 0; i < bones.Count; i++)
                names[i] = pack.ReadString(bones.DataOffset + i * BoneSize)
                    ?? throw new HavokFormatException($"Bone {i} has no name");

            return new HavokSkeleton
            {
                Name = pack.ReadString(o + 0x10),
                BoneNames = names,
                ParentIndices = parentIndices,
                ReferencePose = ReadTransforms(pack.Bytes, pose),
            };
        }

        private static HavokAnimationBinding ReadBinding(HavokPackfile pack, HavokObject obj, Dictionary<int, HavokAnimation> animationCache)
        {
            int o = obj.Offset;
            int animationOffset = pack.RequirePointer(o + 0x18, "binding animation");
            if (!animationCache.TryGetValue(animationOffset, out var animation))
            {
                animation = ReadAnimation(pack, pack.RequireObjectAt(animationOffset, "binding animation"));
                animationCache[animationOffset] = animation;
            }

            var trackToBone = pack.ReadArray(o + 0x20, 2, "transformTrackToBoneIndices");
            if (trackToBone.Count != animation.TransformTrackCount)
                throw new HavokFormatException($"Binding maps {trackToBone.Count} tracks but the animation has {animation.TransformTrackCount}");

            var boneIndices = new short[trackToBone.Count];
            for (int i = 0; i < boneIndices.Length; i++)
                boneIndices[i] = pack.Bytes.I16(trackToBone.DataOffset + i * 2);

            int blendHint = pack.Bytes.U8(o + 0x50);
            return new HavokAnimationBinding
            {
                OriginalSkeletonName = pack.ReadString(o + 0x10),
                TransformTrackToBoneIndices = boneIndices,
                BlendHint = Enum.IsDefined(typeof(HavokBlendHint), blendHint)
                    ? (HavokBlendHint)blendHint
                    : throw new HavokFormatException($"Unknown blend hint {blendHint}"),
                Animation = animation,
            };
        }

        private static HavokAnimation ReadAnimation(HavokPackfile pack, HavokObject obj) => obj.ClassName switch
        {
            SplineAnimationClass => ReadSplineAnimation(pack, obj),
            InterleavedAnimationClass => ReadInterleavedAnimation(pack, obj),
            LosslessAnimationClass => ReadLosslessAnimation(pack, obj),
            _ => throw new HavokUnsupportedClassException("animation", obj.ClassName),
        };

        private static HavokSplineAnimation ReadSplineAnimation(HavokPackfile pack, HavokObject obj)
        {
            var b = pack.Bytes;
            int o = obj.Offset;
            int trackCount = CheckedCount(b.I32(o + 0x18), MaxTracks, "transform track count");
            int frameCount = CheckedCount(b.I32(o + 0x38), MaxFrames, "frame count");
            int blockCount = CheckedCount(b.I32(o + 0x3C), MaxFrames, "block count");
            int maxFramesPerBlock = CheckedCount(b.I32(o + 0x40), MaxFrames, "max frames per block");

            var blockOffsets = pack.ReadArray(o + 0x58, 4, "blockOffsets");
            if (blockOffsets.Count != blockCount)
                throw new HavokFormatException($"Block offset count {blockOffsets.Count} differs from block count {blockCount}");
            var data = pack.ReadArray(o + 0x98, 1, "spline data");

            return new HavokSplineAnimation
            {
                ClassName = obj.ClassName,
                Duration = b.F32(o + 0x14),
                TransformTrackCount = trackCount,
                FrameCount = frameCount,
                FrameDuration = b.F32(o + 0x50),
                AnnotationTracks = ReadAnnotationTracks(pack, o + 0x28),
                BlockCount = blockCount,
                MaxFramesPerBlock = maxFramesPerBlock,
                MaskAndQuantizationSize = b.I32(o + 0x44),
                BlockOffsets = Enumerable.Range(0, blockOffsets.Count).Select(i => b.U32(blockOffsets.DataOffset + i * 4)).ToArray(),
                Data = b.Copy(data.DataOffset, data.Count, "spline data"),
            };
        }

        private static HavokInterleavedAnimation ReadInterleavedAnimation(HavokPackfile pack, HavokObject obj)
        {
            var b = pack.Bytes;
            int o = obj.Offset;
            int trackCount = CheckedCount(b.I32(o + 0x18), MaxTracks, "transform track count");
            var transforms = pack.ReadArray(o + 0x38, QsTransformSize, "interleaved transforms");
            if (trackCount == 0 || transforms.Count % trackCount != 0)
                throw new HavokFormatException($"{transforms.Count} interleaved transforms do not divide into {trackCount} tracks");

            int frameCount = transforms.Count / trackCount;
            float duration = b.F32(o + 0x14);
            return new HavokInterleavedAnimation
            {
                ClassName = obj.ClassName,
                Duration = duration,
                TransformTrackCount = trackCount,
                FrameCount = frameCount,
                FrameDuration = frameCount > 1 ? duration / (frameCount - 1) : 0f,
                AnnotationTracks = ReadAnnotationTracks(pack, o + 0x28),
                Transforms = ReadTransforms(b, transforms),
            };
        }

        private static HavokLosslessAnimation ReadLosslessAnimation(HavokPackfile pack, HavokObject obj)
        {
            var b = pack.Bytes;
            int o = obj.Offset;
            int trackCount = CheckedCount(b.I32(o + 0x18), MaxTracks, "transform track count");
            int floatTrackCount = CheckedCount(b.I32(o + 0x1C), MaxTracks, "float track count");
            if (floatTrackCount != 0)
                throw new HavokUnsupportedClassException($"{obj.ClassName} with {floatTrackCount} float tracks", obj.ClassName);
            int frameCount = CheckedCount(b.I32(o + 0xD8), MaxFrames, "frame count");
            float duration = b.F32(o + 0x14);

            return new HavokLosslessAnimation
            {
                ClassName = obj.ClassName,
                Duration = duration,
                TransformTrackCount = trackCount,
                FrameCount = frameCount,
                FrameDuration = frameCount > 1 ? duration / (frameCount - 1) : 0f,
                AnnotationTracks = ReadAnnotationTracks(pack, o + 0x28),
                DynamicTranslations = ReadFloats(pack, o + 0x38, "dynamic translations"),
                StaticTranslations = ReadFloats(pack, o + 0x48, "static translations"),
                TranslationDescriptors = ReadUInt16s(pack, o + 0x58, trackCount, LosslessVectorDescriptorShorts, "translation descriptors"),
                DynamicRotations = ReadQuaternions(pack, o + 0x68, "dynamic rotations"),
                StaticRotations = ReadQuaternions(pack, o + 0x78, "static rotations"),
                RotationDescriptors = ReadUInt16s(pack, o + 0x88, trackCount, 1, "rotation descriptors"),
                DynamicScales = ReadFloats(pack, o + 0x98, "dynamic scales"),
                StaticScales = ReadFloats(pack, o + 0xA8, "static scales"),
                ScaleDescriptors = ReadUInt16s(pack, o + 0xB8, trackCount, LosslessVectorDescriptorShorts, "scale descriptors"),
            };
        }

        private static float[] ReadFloats(HavokPackfile pack, int arrayField, string what)
        {
            var array = pack.ReadArray(arrayField, sizeof(float), what);
            return Enumerable.Range(0, array.Count).Select(i => pack.Bytes.F32(array.DataOffset + i * sizeof(float))).ToArray();
        }

        private static Quaternion[] ReadQuaternions(HavokPackfile pack, int arrayField, string what)
        {
            var array = pack.ReadArray(arrayField, QuaternionSize, what);
            var b = pack.Bytes;
            return Enumerable.Range(0, array.Count).Select(i =>
            {
                int q = array.DataOffset + i * QuaternionSize;
                return new Quaternion(b.F32(q), b.F32(q + 4), b.F32(q + 8), b.F32(q + 12));
            }).ToArray();
        }

        /// <summary>Reads <paramref name="trackCount"/> elements of <paramref name="shortsPerTrack"/> u16 each, flattened.</summary>
        private static ushort[] ReadUInt16s(HavokPackfile pack, int arrayField, int trackCount, int shortsPerTrack, string what)
        {
            var array = pack.ReadArray(arrayField, shortsPerTrack * sizeof(ushort), what);
            if (array.Count != trackCount)
                throw new HavokFormatException($"{what}: {array.Count} entries, expected {trackCount}");
            return Enumerable.Range(0, array.Count * shortsPerTrack).Select(i => pack.Bytes.U16(array.DataOffset + i * sizeof(ushort))).ToArray();
        }

        private static List<HavokAnnotationTrack> ReadAnnotationTracks(HavokPackfile pack, int arrayField)
        {
            var tracks = pack.ReadArray(arrayField, AnnotationTrackSize, "annotationTracks");
            var result = new List<HavokAnnotationTrack>(tracks.Count);
            for (int t = 0; t < tracks.Count; t++)
            {
                int track = tracks.DataOffset + t * AnnotationTrackSize;
                var annotations = pack.ReadArray(track + 0x08, AnnotationSize, "annotations");
                var list = new List<HavokAnnotation>(annotations.Count);
                for (int a = 0; a < annotations.Count; a++)
                {
                    int annotation = annotations.DataOffset + a * AnnotationSize;
                    list.Add(new HavokAnnotation(pack.Bytes.F32(annotation), pack.ReadString(annotation + 0x08) ?? ""));
                }
                result.Add(new HavokAnnotationTrack(pack.ReadString(track) ?? "", list));
            }
            return result;
        }

        private static HavokTransform[] ReadTransforms(HavokBytes b, HavokArrayRef array)
        {
            var result = new HavokTransform[array.Count];
            for (int i = 0; i < result.Length; i++)
            {
                int o = array.DataOffset + i * QsTransformSize;
                result[i] = new HavokTransform(
                    new Vector3(b.F32(o), b.F32(o + 4), b.F32(o + 8)),
                    new Quaternion(b.F32(o + 16), b.F32(o + 20), b.F32(o + 24), b.F32(o + 28)),
                    new Vector3(b.F32(o + 32), b.F32(o + 36), b.F32(o + 40)));
            }
            return result;
        }

        private static int CheckedCount(int value, int max, string what)
        {
            if (value < 0 || value > max)
                throw new HavokFormatException($"Implausible {what}: {value}");
            return value;
        }
    }
}
