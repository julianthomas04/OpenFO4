using System.Numerics;
using AssetConverter.Havok;
using Xunit;

namespace InstallerTests.Havok
{
    /// <summary>One spline-compressed block: masks first, then channel data aligned relative to the block start.</summary>
    internal sealed class SplineBlockWriter
    {
        private readonly List<byte> _bytes = new();

        public int Position => _bytes.Count;

        public SplineBlockWriter Mask(byte quantization, byte positionFlags, byte rotationFlags, byte scaleFlags)
        {
            _bytes.AddRange([quantization, positionFlags, rotationFlags, scaleFlags]);
            return this;
        }

        public SplineBlockWriter U8(params byte[] values)
        {
            _bytes.AddRange(values);
            return this;
        }

        public SplineBlockWriter U16(ushort value)
        {
            _bytes.AddRange(BitConverter.GetBytes(value));
            return this;
        }

        public SplineBlockWriter F32(params float[] values)
        {
            foreach (float v in values)
                _bytes.AddRange(BitConverter.GetBytes(v));
            return this;
        }

        public SplineBlockWriter Align(int alignment)
        {
            while (_bytes.Count % alignment != 0)
                _bytes.Add(0xCD);
            return this;
        }

        /// <summary>u16 numControlPoints - 1, u8 degree, then numControlPoints + degree + 1 u8 knots.</summary>
        public SplineBlockWriter SplineHeader(int degree, params byte[] knots)
        {
            int controlPoints = knots.Length - degree - 1;
            return U16((ushort)(controlPoints - 1)).U8((byte)degree).U8(knots);
        }

        public byte[] ToArray() => _bytes.ToArray();
    }

    public class HavokSplineAnimationDecoderTests
    {
        private const byte PositionBits8 = 0;
        private const byte RotationThreeComp40 = 1 << 2;
        private const byte RotationUncompressed = 5 << 2;
        private const byte RotationPolar32 = 0 << 2;
        private const byte StaticXyz = 0x07;
        private const byte SplineX = 0x10;
        private const byte StaticRotation = 0x01;
        private const byte SplineRotation = 0x10;
        private const float Tolerance = 1e-4f;
        private static readonly byte[] IdentityThreeComp40 = [0xFF, 0xF7, 0x7F, 0xFF, 0x37];
        private static readonly float[] TwoBlockExpectedX = [1f, 1f, 0f, 2f, 4f];

        [Fact]
        public void SampleTracks_StaticChannels_RepeatTheStoredValueOnEveryFrame()
        {
            var block = new SplineBlockWriter()
                .Mask(PositionBits8 | RotationThreeComp40, StaticXyz, StaticRotation, 0)
                .Align(4).F32(1f, 2f, 3f)
                .U8(IdentityThreeComp40).Align(4);
            var animation = Animation(frames: 3, tracks: 1, maxFramesPerBlock: 4, blocks: [block.ToArray()]);

            var track = Assert.Single(animation.SampleTracks());

            Assert.All(track, t =>
            {
                Assert.Equal(new Vector3(1f, 2f, 3f), t.Translation);
                AssertRotation(Quaternion.Identity, t.Rotation);
                Assert.Equal(Vector3.One, t.Scale);
            });
        }

        [Fact]
        public void SampleTracks_SplineAxisWithEightBitControlPoints_InterpolatesBetweenMinAndMax()
        {
            // Linear spline over local frames 0..2: raw 0 -> min (10), raw 255 -> max (20); y and z are neither static nor spline -> 0.
            var block = new SplineBlockWriter()
                .Mask(PositionBits8, SplineX, 0, 0)
                .SplineHeader(degree: 1, 0, 0, 2, 2).Align(4)
                .F32(10f, 20f)
                .U8(0, 255).Align(4);
            var animation = Animation(frames: 3, tracks: 1, maxFramesPerBlock: 4, blocks: [block.ToArray()]);

            var track = Assert.Single(animation.SampleTracks());

            Assert.Equal(10f, track[0].Translation.X, Tolerance);
            Assert.Equal(15f, track[1].Translation.X, Tolerance);
            Assert.Equal(20f, track[2].Translation.X, Tolerance);
            Assert.Equal(0f, track[1].Translation.Y);
        }

        [Fact]
        public void SampleTracks_SixteenBitControlPoints_UseTheFullU16Range()
        {
            var block = new SplineBlockWriter()
                .Mask(1, SplineX, 0, 0)
                .SplineHeader(1, 0, 0, 2, 2).Align(4)
                .F32(-1f, 1f)
                .U16(0).U16(65535).Align(4);
            var animation = Animation(frames: 3, tracks: 1, maxFramesPerBlock: 4, blocks: [block.ToArray()]);

            var track = Assert.Single(animation.SampleTracks());

            Assert.Equal(-1f, track[0].Translation.X, Tolerance);
            Assert.Equal(0f, track[1].Translation.X, Tolerance);
            Assert.Equal(1f, track[2].Translation.X, Tolerance);
        }

        [Fact]
        public void SampleTracks_UncompressedRotationSpline_InterpolatesAndRenormalizes()
        {
            // Identity to 90 degrees about Z over two frames: the midpoint is 45 degrees, (0, 0, sin 22.5, cos 22.5).
            float s = MathF.Sqrt(0.5f);
            var block = new SplineBlockWriter()
                .Mask(PositionBits8 | RotationUncompressed, 0, SplineRotation, 0)
                .SplineHeader(1, 0, 0, 2, 2).Align(4)
                .F32(0, 0, 0, 1)
                .F32(0, 0, s, s).Align(4);
            var animation = Animation(frames: 3, tracks: 1, maxFramesPerBlock: 4, blocks: [block.ToArray()]);

            var track = Assert.Single(animation.SampleTracks());

            AssertRotation(Quaternion.Identity, track[0].Rotation);
            AssertRotation(new Quaternion(0f, 0f, 0.3826834f, 0.9238795f), track[1].Rotation);
            AssertRotation(new Quaternion(0f, 0f, s, s), track[2].Rotation);
        }

        [Fact]
        public void SampleTracks_TwoBlocks_ShareTheirBoundaryFrameAndRestartLocalTime()
        {
            // maxFramesPerBlock 3 -> blocks start at frames 0 and 2. Block 0 holds x = 1; block 1 is a spline 0 -> 4 over local frames 0..2 = global 2..4.
            var staticBlock = new SplineBlockWriter()
                .Mask(PositionBits8, StaticXyz, 0, 0)
                .Align(4).F32(1f, 0f, 0f);
            var splineBlock = new SplineBlockWriter()
                .Mask(PositionBits8, SplineX, 0, 0)
                .SplineHeader(1, 0, 0, 2, 2).Align(4)
                .F32(0f, 4f)
                .U8(0, 255).Align(4);
            var animation = Animation(frames: 5, tracks: 1, maxFramesPerBlock: 3, blocks: [staticBlock.ToArray(), splineBlock.ToArray()]);

            var track = Assert.Single(animation.SampleTracks());

            Assert.Equal(TwoBlockExpectedX, track.Select(t => t.Translation.X).ToArray(), new FloatComparer(Tolerance));
        }

        [Fact]
        public void SampleTracks_SeveralTracks_ReadsChannelsInTrackOrder()
        {
            var block = new SplineBlockWriter()
                .Mask(PositionBits8, StaticXyz, 0, 0)
                .Mask(PositionBits8, StaticXyz, 0, 0)
                .Align(4).F32(1f, 1f, 1f)
                .Align(4).F32(2f, 2f, 2f);
            var animation = Animation(frames: 2, tracks: 2, maxFramesPerBlock: 4, blocks: [block.ToArray()]);

            var tracks = animation.SampleTracks();

            Assert.Equal(new Vector3(1f), tracks[0][1].Translation);
            Assert.Equal(new Vector3(2f), tracks[1][1].Translation);
        }

        [Fact]
        public void SampleTracks_Polar32Rotation_ThrowsUnsupportedQuantization()
        {
            var block = new SplineBlockWriter()
                .Mask(RotationPolar32, 0, StaticRotation, 0)
                .Align(4).U8(0, 0, 0, 0);
            var animation = Animation(frames: 2, tracks: 1, maxFramesPerBlock: 4, blocks: [block.ToArray()]);

            Assert.Throws<HavokUnsupportedQuantizationException>(() => animation.SampleTracks());
        }

        [Fact]
        public void SampleTracks_UndefinedVectorPrecision_ThrowsUnsupportedQuantization()
        {
            var block = new SplineBlockWriter()
                .Mask(2, SplineX, 0, 0)
                .SplineHeader(1, 0, 0, 2, 2).Align(4).F32(0f, 1f).U8(0, 0, 0, 0);
            var animation = Animation(frames: 3, tracks: 1, maxFramesPerBlock: 4, blocks: [block.ToArray()]);

            Assert.Throws<HavokUnsupportedQuantizationException>(() => animation.SampleTracks());
        }

        [Fact]
        public void SampleTracks_ChannelDataRunsPastTheBlock_ThrowsFormatException()
        {
            var block = new SplineBlockWriter()
                .Mask(PositionBits8, StaticXyz, 0, 0)
                .Align(4).F32(1f, 2f, 3f);
            var animation = Animation(frames: 2, tracks: 1, maxFramesPerBlock: 4, blocks: [block.ToArray()[..10]]);

            Assert.Throws<HavokFormatException>(() => animation.SampleTracks());
        }

        [Fact]
        public void SampleTracks_TrackDataOverrunsIntoTheNextBlock_ThrowsFormatException()
        {
            var firstBlock = new SplineBlockWriter()
                .Mask(PositionBits8, SplineX, 0, 0)
                .SplineHeader(1, 0, 0, 2, 2).Align(4).F32(0f, 1f).U8(0, 255).Align(4);
            var secondBlock = new SplineBlockWriter()
                .Mask(PositionBits8, StaticXyz, 0, 0)
                .Align(4).F32(1f, 2f, 3f);
            const uint secondBlockStartInsideTheFirstBlocksData = 10;
            var animation = Animation(frames: 5, tracks: 1, maxFramesPerBlock: 3,
                blocks: [firstBlock.ToArray(), secondBlock.ToArray()], blockOffsets: [0, secondBlockStartInsideTheFirstBlocksData]);

            Assert.Throws<HavokFormatException>(() => animation.SampleTracks());
        }

        [Fact]
        public void SampleTracks_MaskAreaSmallerThanTheTrackMasks_ThrowsFormatException()
        {
            var animation = Animation(frames: 2, tracks: 2, maxFramesPerBlock: 4, blocks: [new byte[16]], maskSize: 4);

            Assert.Throws<HavokFormatException>(() => animation.SampleTracks());
        }

        [Theory]
        [InlineData(1)]
        [InlineData(0)]
        public void SampleTracks_BlockSizeBelowTwoFrames_ThrowsFormatException(int maxFramesPerBlock)
        {
            var animation = Animation(frames: 2, tracks: 1, maxFramesPerBlock: maxFramesPerBlock, blocks: [new byte[16]]);

            Assert.Throws<HavokFormatException>(() => animation.SampleTracks());
        }

        [Fact]
        public void SampleTracks_BlocksCannotCoverTheFrames_ThrowsFormatException()
        {
            // One block of 4 frames covers frames 0..3 only.
            var animation = Animation(frames: 9, tracks: 1, maxFramesPerBlock: 4, blocks: [new byte[16]]);

            Assert.Throws<HavokFormatException>(() => animation.SampleTracks());
        }

        [Fact]
        public void SampleTracks_FramesTimesTracksBeyondTheBudget_ThrowsBeforeAllocating()
        {
            var animation = Animation(frames: 1 << 20, tracks: 1 << 14, maxFramesPerBlock: 1 << 20, blocks: [new byte[16]]);

            Assert.Throws<HavokFormatException>(() => animation.SampleTracks());
        }

        [Fact]
        public void SampleTracks_UnclampedSplineWhoseKnotsEndBeforeTheLastFrame_HoldsTheFinalControlPoint()
        {
            // Knots stop at local frame 1 but the block spans frames 0..3: later frames must hold the end value instead of extrapolating.
            var block = new SplineBlockWriter()
                .Mask(PositionBits8, SplineX, 0, 0)
                .SplineHeader(1, 0, 0, 1, 1).Align(4)
                .F32(0f, 10f)
                .U8(0, 255).Align(4);
            var animation = Animation(frames: 4, tracks: 1, maxFramesPerBlock: 4, blocks: [block.ToArray()]);

            var track = Assert.Single(animation.SampleTracks());

            Assert.Equal(10f, track[3].Translation.X, Tolerance);
        }

        private static HavokSplineAnimation Animation(int frames, int tracks, int maxFramesPerBlock, byte[][] blocks,
            uint[]? blockOffsets = null, int? maskSize = null)
        {
            var offsets = blockOffsets ?? blocks.Select((_, i) => (uint)blocks.Take(i).Sum(b => b.Length)).ToArray();
            return new HavokSplineAnimation
            {
                ClassName = "hkaSplineCompressedAnimation",
                TransformTrackCount = tracks,
                FrameCount = frames,
                FrameDuration = 1f / 30f,
                Duration = (frames - 1) / 30f,
                AnnotationTracks = [],
                BlockCount = blocks.Length,
                MaxFramesPerBlock = maxFramesPerBlock,
                MaskAndQuantizationSize = maskSize ?? tracks * 4,
                BlockOffsets = offsets,
                Data = blocks.SelectMany(b => b).ToArray(),
            };
        }

        private static void AssertRotation(Quaternion expected, Quaternion actual)
        {
            Assert.InRange(MathF.Abs(Quaternion.Dot(expected, actual)), 1f - 1e-5f, 1f + 1e-5f);
        }

        private sealed class FloatComparer(float tolerance) : IEqualityComparer<float>
        {
            public bool Equals(float x, float y) => MathF.Abs(x - y) <= tolerance;

            public int GetHashCode(float obj) => 0;
        }
    }

    public class HavokLosslessAnimationDecoderTests
    {
        private const ushort Default = 0;

        [Fact]
        public void SampleTracks_StaticDynamicAndDefaultDescriptors_PickTheMatchingStores()
        {
            // 2 tracks, 3 frames. Track 0: static translation (7, 8, 9), dynamic rotation channel 0, default scale.
            // Track 1: translation x dynamic channel 0, y default, z static index 0; rotation static; scale static (2, 2, 2).
            var animation = Animation(frames: 3, tracks: 2,
                dynamicTranslations: [10f, 11f, 12f],
                staticTranslations: [7f, 8f, 9f],
                translationDescriptors:
                [
                    Static(0), Static(1), Static(2), 0,
                    Dynamic(0), Default, Static(0), 0,
                ],
                dynamicRotations: [Z(0.1f), Z(0.2f), Z(0.3f)],
                staticRotations: [new Quaternion(0f, 0f, 0f, 3f)],
                rotationDescriptors: [Dynamic(0), Static(0)],
                staticScales: [2f],
                scaleDescriptors:
                [
                    Default, Default, Default, 0,
                    Static(0), Static(0), Static(0), 0,
                ]);

            var tracks = animation.SampleTracks();

            Assert.Equal(new Vector3(7f, 8f, 9f), tracks[0][2].Translation);
            Assert.Equal(Vector3.One, tracks[0][0].Scale);
            Assert.Equal(new Vector3(11f, 0f, 7f), tracks[1][1].Translation);
            Assert.Equal(new Vector3(2f), tracks[1][0].Scale);
            Assert.Equal(Quaternion.Identity, tracks[1][0].Rotation);
        }

        [Fact]
        public void SampleTracks_DynamicRotation_IsReadFrameMajorAndNormalized()
        {
            var animation = Animation(frames: 3, tracks: 1,
                dynamicRotations: [Z(0.1f, 2f), Z(0.2f, 2f), Z(0.3f, 2f)],
                rotationDescriptors: [Dynamic(0)]);

            var track = Assert.Single(animation.SampleTracks());

            Assert.Equal(0.2f / MathF.Sqrt(0.2f * 0.2f + 4f), track[1].Rotation.Z, 1e-6f);
            Assert.All(track, t => Assert.Equal(1f, t.Rotation.Length(), 1e-6f));
        }

        [Fact]
        public void SampleTracks_TwoDynamicChannels_AreInterleavedPerFrame()
        {
            // channelCount = dynamic.Length / numFrames = 2; value of channel c at frame f is dynamic[f * 2 + c].
            var animation = Animation(frames: 2, tracks: 1,
                dynamicTranslations: [1f, 2f, 3f, 4f],
                translationDescriptors: [Dynamic(0), Dynamic(1), Default, 0],
                rotationDescriptors: [Default]);

            var track = Assert.Single(animation.SampleTracks());

            Assert.Equal(new Vector3(1f, 2f, 0f), track[0].Translation);
            Assert.Equal(new Vector3(3f, 4f, 0f), track[1].Translation);
        }

        [Fact]
        public void SampleTracks_StaticIndexOutsideTheStore_ThrowsFormatException()
        {
            var animation = Animation(frames: 2, tracks: 1,
                staticTranslations: [1f],
                translationDescriptors: [Static(5), Default, Default, 0],
                rotationDescriptors: [Default]);

            Assert.Throws<HavokFormatException>(() => animation.SampleTracks());
        }

        [Fact]
        public void SampleTracks_DefaultDescriptorWithAnIndex_ThrowsFormatException()
        {
            var animation = Animation(frames: 2, tracks: 1,
                translationDescriptors: [4, Default, Default, 0],
                rotationDescriptors: [Default]);

            Assert.Throws<HavokFormatException>(() => animation.SampleTracks());
        }

        [Fact]
        public void SampleTracks_DynamicValuesNotDivisibleByFrames_ThrowsFormatException()
        {
            var animation = Animation(frames: 3, tracks: 1,
                dynamicTranslations: [1f, 2f, 3f, 4f],
                translationDescriptors: [Dynamic(0), Default, Default, 0],
                rotationDescriptors: [Default]);

            Assert.Throws<HavokFormatException>(() => animation.SampleTracks());
        }

        [Fact]
        public void SampleTracks_FewerDescriptorsThanTracksNeed_ThrowsFormatException()
        {
            var animation = Animation(frames: 2, tracks: 2,
                translationDescriptors: [Default, Default, Default, 0],
                rotationDescriptors: [Default, Default]);

            Assert.Throws<HavokFormatException>(() => animation.SampleTracks());
        }

        [Fact]
        public void SampleTracks_ZeroRotation_ThrowsFormatException()
        {
            var animation = Animation(frames: 2, tracks: 1,
                staticRotations: [Quaternion.Zero],
                rotationDescriptors: [Static(0)]);

            Assert.Throws<HavokFormatException>(() => animation.SampleTracks());
        }

        [Fact]
        public void SampleTracks_UnknownDescriptorKind_ThrowsFormatException()
        {
            var animation = Animation(frames: 2, tracks: 1,
                translationDescriptors: [3, Default, Default, 0],
                rotationDescriptors: [Default]);

            Assert.Throws<HavokFormatException>(() => animation.SampleTracks());
        }

        private static ushort Static(int index) => (ushort)((index << 2) | 1);

        private static ushort Dynamic(int index) => (ushort)((index << 2) | 2);

        private static Quaternion Z(float z, float w = 1f) => new(0f, 0f, z, w);

        private static HavokLosslessAnimation Animation(int frames, int tracks,
            float[]? dynamicTranslations = null, float[]? staticTranslations = null, ushort[]? translationDescriptors = null,
            Quaternion[]? dynamicRotations = null, Quaternion[]? staticRotations = null, ushort[]? rotationDescriptors = null,
            float[]? dynamicScales = null, float[]? staticScales = null, ushort[]? scaleDescriptors = null) => new()
            {
                ClassName = "hkaLosslessCompressedAnimation",
                TransformTrackCount = tracks,
                FrameCount = frames,
                FrameDuration = 1f / 30f,
                Duration = (frames - 1) / 30f,
                AnnotationTracks = [],
                DynamicTranslations = dynamicTranslations ?? [],
                StaticTranslations = staticTranslations ?? [],
                TranslationDescriptors = translationDescriptors ?? new ushort[tracks * 4],
                DynamicRotations = dynamicRotations ?? [],
                StaticRotations = staticRotations ?? [],
                RotationDescriptors = rotationDescriptors ?? new ushort[tracks],
                DynamicScales = dynamicScales ?? [],
                StaticScales = staticScales ?? [],
                ScaleDescriptors = scaleDescriptors ?? new ushort[tracks * 4],
            };
    }
}
