using System.Numerics;

namespace AssetConverter.Havok
{
    public readonly record struct HavokTransform(Vector3 Translation, Quaternion Rotation, Vector3 Scale)
    {
        public static readonly HavokTransform Identity = new(Vector3.Zero, Quaternion.Identity, Vector3.One);
    }

    public sealed record HavokAnnotation(float Time, string Text);

    public sealed record HavokAnnotationTrack(string TrackName, IReadOnlyList<HavokAnnotation> Annotations);

    public sealed class HavokSkeleton
    {
        public string? Name { get; init; }
        public required IReadOnlyList<string> BoneNames { get; init; }
        public required IReadOnlyList<short> ParentIndices { get; init; }
        public required IReadOnlyList<HavokTransform> ReferencePose { get; init; }
    }

    public enum HavokBlendHint
    {
        Normal = 0,
        Additive = 1,
        AdditiveDeprecated = 2,
    }

    public sealed class HavokAnimationBinding
    {
        public string? OriginalSkeletonName { get; init; }
        public required IReadOnlyList<short> TransformTrackToBoneIndices { get; init; }
        public HavokBlendHint BlendHint { get; init; }
        public required HavokAnimation Animation { get; init; }
    }

    public abstract class HavokAnimation
    {
        public required string ClassName { get; init; }
        public float Duration { get; init; }
        public int TransformTrackCount { get; init; }
        public int FrameCount { get; init; }
        public float FrameDuration { get; init; }
        public required IReadOnlyList<HavokAnnotationTrack> AnnotationTracks { get; init; }

        /// <summary>Samples every transform track at every frame: result[track][frame].</summary>
        public abstract HavokTransform[][] SampleTracks();
    }

    public sealed class HavokInterleavedAnimation : HavokAnimation
    {
        public required IReadOnlyList<HavokTransform> Transforms { get; init; }

        public override HavokTransform[][] SampleTracks()
        {
            var tracks = new HavokTransform[TransformTrackCount][];
            for (int t = 0; t < TransformTrackCount; t++)
            {
                tracks[t] = new HavokTransform[FrameCount];
                for (int f = 0; f < FrameCount; f++)
                    tracks[t][f] = Transforms[f * TransformTrackCount + t];
            }
            return tracks;
        }
    }

    public sealed class HavokSplineAnimation : HavokAnimation
    {
        public int BlockCount { get; init; }
        public int MaxFramesPerBlock { get; init; }
        public int MaskAndQuantizationSize { get; init; }
        public required IReadOnlyList<uint> BlockOffsets { get; init; }
        public required byte[] Data { get; init; }

        public override HavokTransform[][] SampleTracks() => SplineAnimationDecoder.Decode(this);
    }

    /// <summary>
    /// hkaLosslessCompressedAnimation: per track and component a descriptor <c>(index &lt;&lt; 2) | kind</c> selects a default value,
    /// a static value or one of the dynamic channels that hold one value per frame (frame-major).
    /// </summary>
    public sealed class HavokLosslessAnimation : HavokAnimation
    {
        public required IReadOnlyList<float> DynamicTranslations { get; init; }
        public required IReadOnlyList<float> StaticTranslations { get; init; }
        /// <summary>Four u16 per track (x, y, z, padding).</summary>
        public required IReadOnlyList<ushort> TranslationDescriptors { get; init; }
        public required IReadOnlyList<Quaternion> DynamicRotations { get; init; }
        public required IReadOnlyList<Quaternion> StaticRotations { get; init; }
        /// <summary>One u16 per track.</summary>
        public required IReadOnlyList<ushort> RotationDescriptors { get; init; }
        public required IReadOnlyList<float> DynamicScales { get; init; }
        public required IReadOnlyList<float> StaticScales { get; init; }
        /// <summary>Four u16 per track (x, y, z, padding).</summary>
        public required IReadOnlyList<ushort> ScaleDescriptors { get; init; }

        public override HavokTransform[][] SampleTracks() => LosslessAnimationDecoder.Decode(this);
    }

    public sealed record HavokClipTrack(int BoneIndex, HavokTransform[] Frames);

    public sealed record HavokClip
    {
        public required string Name { get; init; }
        public float Duration { get; init; }
        public int FrameCount { get; init; }
        public float FrameDuration { get; init; }
        public HavokBlendHint BlendHint { get; init; }
        public string? OriginalSkeletonName { get; init; }
        public required IReadOnlyList<HavokClipTrack> Tracks { get; init; }
        public required IReadOnlyList<HavokAnnotation> Annotations { get; init; }
    }
}
