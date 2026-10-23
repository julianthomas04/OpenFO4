using System.Numerics;
using AssetConverter.Havok;
using Xunit;

namespace InstallerTests.Havok
{
    // Object field offsets used by the reader.
    public class HavokAnimationFileReaderTests
    {
        private static readonly float[] LosslessExpectedX = [1f, 2f, 3f, 4f];

        private static readonly HkBone[] Bones =
        [
            new("Root", -1, new Vector3(0, 0, 10), Quaternion.Identity, Vector3.One),
            new("Spine", 0, new Vector3(0, 5, 0), new Quaternion(0f, 0f, 0.7071068f, 0.7071068f), Vector3.One),
            new("Head", 1, new Vector3(1, 2, 3), Quaternion.Identity, new Vector3(2, 2, 2)),
        ];

        [Fact]
        public void ReadFile_Skeleton_ReadsNamesParentsAndReferencePose()
        {
            var file = HavokClipBuilder.ReadFile(HkaFile.Build([("Root", Bones)], []));

            var skeleton = Assert.Single(file.Skeletons);
            Assert.Equal("Root", skeleton.Name);
            Assert.Equal(["Root", "Spine", "Head"], skeleton.BoneNames);
            Assert.Equal(new short[] { -1, 0, 1 }, skeleton.ParentIndices);
            Assert.Equal(new Vector3(1, 2, 3), skeleton.ReferencePose[2].Translation);
            Assert.Equal(new Vector3(2, 2, 2), skeleton.ReferencePose[2].Scale);
            Assert.Equal(0.7071068f, skeleton.ReferencePose[1].Rotation.W, 1e-6f);
            Assert.True(file.HasSkeletons);
            Assert.False(file.HasAnimations);
        }

        [Fact]
        public void ReadFile_TwoSkeletons_KeepsFileOrder()
        {
            var file = HavokClipBuilder.ReadFile(HkaFile.Build([("Root", Bones), ("Ragdoll", Bones[..1])], []));

            Assert.Equal(["Root", "Ragdoll"], file.Skeletons.Select(s => s.Name));
        }

        [Fact]
        public void ReadFile_InterleavedBinding_ReadsTracksBlendHintAndSkeletonName()
        {
            var animation = new HkInterleavedSpec(Duration: 2f, Tracks: 2, Frames: 3);
            var binding = new HkBindingSpec("Root", [2, 0], BlendHint: 1, animation);

            var file = HavokClipBuilder.ReadFile(HkaFile.Build([("Root", Bones)], [binding]));

            var read = Assert.Single(file.Bindings);
            Assert.Equal("Root", read.OriginalSkeletonName);
            Assert.Equal(HavokBlendHint.Additive, read.BlendHint);
            Assert.Equal(new short[] { 2, 0 }, read.TransformTrackToBoneIndices);
            Assert.Equal(3, read.Animation.FrameCount);
            Assert.Equal(1f, read.Animation.FrameDuration);
        }

        [Fact]
        public void SampleTracks_InterleavedAnimation_DeinterleavesFrameMajorTransforms()
        {
            var binding = new HkBindingSpec("Root", [0, 1], 0, new HkInterleavedSpec(2f, Tracks: 2, Frames: 3));

            var tracks = HavokClipBuilder.ReadFile(HkaFile.Build([("Root", Bones)], [binding])).Bindings[0].Animation.SampleTracks();

            Assert.Equal(new Vector3(2f, 1f, 0f), tracks[1][2].Translation);
            Assert.Equal(new Vector3(1f, 0f, 0f), tracks[0][1].Translation);
        }

        [Fact]
        public void ReadFile_BindingsSharingOneAnimation_ShareTheDecodedObject()
        {
            var animation = new HkInterleavedSpec(1f, 1, 2);
            var first = new HkBindingSpec("Root", [0], 0, animation);
            var second = new HkBindingSpec("Root", [1], 0, animation);

            var file = HavokClipBuilder.ReadFile(HkaFile.Build([("Root", Bones)], [first, second]));

            Assert.Same(file.Bindings[0].Animation, file.Bindings[1].Animation);
        }

        [Fact]
        public void ReadFile_AnnotationTracks_AreReadWithTimesAndText()
        {
            var annotations = new[] { new HkAnnotationSpec("Events", [(0.5f, "FootLeft"), (1.25f, "FootRight")]) };
            var animation = new HkInterleavedSpec(2f, 1, 2, annotations);
            var binding = new HkBindingSpec("Root", [0], 0, animation);

            var file = HavokClipBuilder.ReadFile(HkaFile.Build([("Root", Bones)], [binding]));

            var track = Assert.Single(file.Bindings[0].Animation.AnnotationTracks);
            Assert.Equal("Events", track.TrackName);
            Assert.Equal([new HavokAnnotation(0.5f, "FootLeft"), new HavokAnnotation(1.25f, "FootRight")], track.Annotations);
        }

        [Fact]
        public void ReadFile_SplineAnimation_ReadsFrameGeometryAndDecodesThroughTheSplineDecoder()
        {
            var block = new SplineBlockWriter()
                .Mask(0, 0x07, 0, 0)
                .Align(4).F32(4f, 5f, 6f);
            var spline = new HkSplineSpec(Duration: 1f, Tracks: 1, Frames: 4, Blocks: 1, MaxFramesPerBlock: 4, MaskSize: 4,
                FrameDuration: 0.25f, BlockData: [block.ToArray()]);
            var binding = new HkBindingSpec("Root", [1], 0, spline);

            var animation = HavokClipBuilder.ReadFile(HkaFile.Build([("Root", Bones)], [binding])).Bindings[0].Animation;

            Assert.Equal(4, animation.FrameCount);
            Assert.Equal(0.25f, animation.FrameDuration);
            Assert.Equal(new Vector3(4f, 5f, 6f), Assert.Single(animation.SampleTracks())[3].Translation);
        }

        [Fact]
        public void ReadFile_LosslessAnimation_UsesDurationOverFramesMinusOne()
        {
            var lossless = new HkLosslessSpec(Duration: 1.5f, Tracks: 1, Frames: 4,
                DynamicTranslations: [1f, 2f, 3f, 4f], StaticTranslations: [],
                TranslationDescriptors: [(2 << 0) | 2, 0, 0, 0], DynamicRotations: [], StaticRotations: [],
                RotationDescriptors: [0]);
            var binding = new HkBindingSpec("Root", [0], 0, lossless);

            var animation = HavokClipBuilder.ReadFile(HkaFile.Build([("Root", Bones)], [binding])).Bindings[0].Animation;

            Assert.Equal(0.5f, animation.FrameDuration);
            Assert.Equal(LosslessExpectedX, Assert.Single(animation.SampleTracks()).Select(t => t.Translation.X).ToArray());
        }

        [Fact]
        public void ReadFile_LosslessAnimationWithFloatTracks_ThrowsUnsupportedClass()
        {
            var lossless = new HkLosslessSpec(1f, 1, 2, [], [], [0, 0, 0, 0], [], [], [0], FloatTracks: 2);
            var binding = new HkBindingSpec("Root", [0], 0, lossless);

            Assert.Throws<HavokUnsupportedClassException>(() => HavokClipBuilder.ReadFile(HkaFile.Build([("Root", Bones)], [binding])));
        }

        [Fact]
        public void TryReadFile_PackfileWithoutAnimationContainer_ReturnsFalse()
        {
            bool found = HavokClipBuilder.TryReadFile(HkaFile.BuildWithoutAnimationContainer(), out var file);

            Assert.False(found);
            Assert.Null(file);
        }

        [Fact]
        public void ReadFile_PackfileWithoutAnimationContainer_ThrowsFormatException()
        {
            Assert.Throws<HavokFormatException>(() => HavokClipBuilder.ReadFile(HkaFile.BuildWithoutAnimationContainer()));
        }

        [Fact]
        public void ReadFile_UnknownBlendHint_ThrowsFormatException()
        {
            var binding = new HkBindingSpec("Root", [0], BlendHint: 9, new HkInterleavedSpec(1f, 1, 2));

            Assert.Throws<HavokFormatException>(() => HavokClipBuilder.ReadFile(HkaFile.Build([("Root", Bones)], [binding])));
        }

        [Fact]
        public void ReadFile_BindingMapsFewerTracksThanTheAnimationHas_ThrowsFormatException()
        {
            var binding = new HkBindingSpec("Root", [0], 0, new HkInterleavedSpec(1f, Tracks: 2, Frames: 2));

            Assert.Throws<HavokFormatException>(() => HavokClipBuilder.ReadFile(HkaFile.Build([("Root", Bones)], [binding])));
        }

        [Fact]
        public void ReadFile_ParentIndexBeyondTheBoneCount_ThrowsFormatException()
        {
            HkBone[] bones = [Bones[0], Bones[1] with { Parent = 7 }];

            Assert.Throws<HavokFormatException>(() => HavokClipBuilder.ReadFile(HkaFile.Build([("Root", bones)], [])));
        }

        [Fact]
        public void ReadFile_SkeletonArraysOfDifferentLengths_ThrowsFormatException()
        {
            var bytes = HkaFile.Build([("Root", Bones)], [], (b, layout) => b.U32(layout.Skeletons[0] + 0x18 + 8, 2));

            Assert.Throws<HavokFormatException>(() => HavokClipBuilder.ReadFile(bytes));
        }

        [Fact]
        public void ReadFile_UnknownAnimationClass_ThrowsUnsupportedClass()
        {
            var binding = new HkBindingSpec("Root", [0], 0, new HkInterleavedSpec(1f, 1, 2));
            var bytes = HkaFile.Build([("Root", Bones)], [binding]).ToArray();
            string interleaved = "hkaInterleavedUncompressedAnimation";
            int at = IndexOf(bytes, System.Text.Encoding.ASCII.GetBytes(interleaved));
            System.Text.Encoding.ASCII.GetBytes("hkaMysteryAnim ").CopyTo(bytes, at);

            var ex = Assert.Throws<HavokUnsupportedClassException>(() => HavokClipBuilder.ReadFile(bytes));
            Assert.StartsWith("hkaMystery", ex.ClassName, StringComparison.Ordinal);
        }

        [Theory]
        [InlineData(1)]
        [InlineData(2)]
        [InlineData(3)]
        [InlineData(4)]
        [InlineData(5)]
        [InlineData(6)]
        [InlineData(7)]
        [InlineData(8)]
        public void ReadFile_CorruptedValidFile_FailsOnlyWithHavokExceptions(int seed)
        {
            var block = new SplineBlockWriter()
                .Mask(1 << 2, 0x07, 0x01, 0)
                .Align(4).F32(4f, 5f, 6f).U8(0xFF, 0xF7, 0x7F, 0xFF, 0x37).Align(4);
            var spline = new HkSplineSpec(1f, 1, 4, 1, 4, 4, 0.25f, [block.ToArray()]);
            var valid = HkaFile.Build([("Root", Bones)],
            [
                new HkBindingSpec("Root", [1], 0, spline),
                new HkBindingSpec("Root", [0, 1], 0, new HkInterleavedSpec(2f, 2, 3, [new HkAnnotationSpec("E", [(0.5f, "x")])])),
            ]);
            var random = new Random(seed);

            for (int attempt = 0; attempt < 400; attempt++)
            {
                var mutated = (byte[])valid.Clone();
                for (int flips = random.Next(1, 4); flips > 0; flips--)
                    mutated[random.Next(mutated.Length)] = (byte)random.Next(256);

                var failure = Record.Exception(() => SampleEverything(mutated));

                Assert.True(failure is null or HavokException, $"attempt {attempt}: {failure}");
            }
        }

        private static void SampleEverything(byte[] bytes)
        {
            foreach (var binding in HavokClipBuilder.ReadFile(bytes).Bindings)
                binding.Animation.SampleTracks();
        }

        private static int IndexOf(byte[] haystack, byte[] needle)
        {
            for (int i = 0; i <= haystack.Length - needle.Length; i++)
                if (haystack.AsSpan(i, needle.Length).SequenceEqual(needle)) return i;
            throw new InvalidOperationException("pattern not found");
        }
    }
}
