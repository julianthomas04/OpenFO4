using System.Numerics;
using AssetConverter.Havok;
using InstallerTests.Archives;
using SharpGLTF.Schema2;
using Xunit;

namespace InstallerTests.Havok
{
    internal static class HavokTestData
    {
        public static readonly Quaternion QuarterTurnAboutZ = new(0f, 0f, MathF.Sqrt(0.5f), MathF.Sqrt(0.5f));

        public static HavokSkeleton Skeleton(params (string Name, short Parent, HavokTransform RestPose)[] bones) => new()
        {
            Name = "Root",
            BoneNames = bones.Select(b => b.Name).ToArray(),
            ParentIndices = bones.Select(b => b.Parent).ToArray(),
            ReferencePose = bones.Select(b => b.RestPose).ToArray(),
        };

        public static HavokSkeleton TwoBoneSkeleton() => Skeleton(
            ("Root", -1, new HavokTransform(new Vector3(0, 0, 10), Quaternion.Identity, Vector3.One)),
            ("Spine", 0, new HavokTransform(new Vector3(0, 5, 0), QuarterTurnAboutZ, new Vector3(2, 3, 4))));

        public static HavokTransform At(Vector3 translation) => new(translation, Quaternion.Identity, Vector3.One);

        public static HavokClip Clip(string name, float frameDuration, params HavokClipTrack[] tracks) => new()
        {
            Name = name,
            Duration = frameDuration * ((tracks.FirstOrDefault()?.Frames.Length ?? 1) - 1),
            FrameCount = tracks.FirstOrDefault()?.Frames.Length ?? 0,
            FrameDuration = frameDuration,
            OriginalSkeletonName = "Root",
            Tracks = tracks,
            Annotations = [],
        };
    }

    public class HavokClipBuilderTests
    {
        [Fact]
        public void Build_BindingTrackMappedToNegativeBone_IsDropped()
        {
            var binding = Binding(trackToBone: [1, -1], tracks: 2, frames: 2);

            var clip = HavokClipBuilder.Build("walk", binding);

            Assert.Equal(1, Assert.Single(clip.Tracks).BoneIndex);
        }

        [Fact]
        public void Build_ConsecutiveQuaternionsOnOppositeHemispheres_AreMadeContinuous()
        {
            var q = HavokTestData.QuarterTurnAboutZ;
            var binding = Binding([0], 1, 3, rotation: f => f == 1 ? Quaternion.Negate(q) : q);

            var clip = HavokClipBuilder.Build("spin", binding);

            Assert.All(clip.Tracks[0].Frames.Zip(clip.Tracks[0].Frames.Skip(1)),
                pair => Assert.True(Quaternion.Dot(pair.First.Rotation, pair.Second.Rotation) > 0f));
        }

        [Fact]
        public void Build_CopiesClipLevelFieldsFromBindingAndAnimation()
        {
            var binding = Binding([0], 1, 4, blendHint: HavokBlendHint.Additive);

            var clip = HavokClipBuilder.Build("aim", binding);

            Assert.Equal("aim", clip.Name);
            Assert.Equal(4, clip.FrameCount);
            Assert.Equal(HavokBlendHint.Additive, clip.BlendHint);
            Assert.Equal("Root", clip.OriginalSkeletonName);
            Assert.Equal(binding.Animation.FrameDuration, clip.FrameDuration);
        }

        [Fact]
        public void Build_AnnotationTracks_AreFlattenedInTimeOrder()
        {
            var binding = Binding([0], 1, 2, annotations:
            [
                new HavokAnnotationTrack("A", [new HavokAnnotation(2f, "late"), new HavokAnnotation(0.5f, "early")]),
                new HavokAnnotationTrack("B", [new HavokAnnotation(1f, "middle")]),
            ]);

            var clip = HavokClipBuilder.Build("x", binding);

            Assert.Equal(["early", "middle", "late"], clip.Annotations.Select(a => a.Text));
        }

        [Fact]
        public void BuildAll_SingleBinding_KeepsTheBaseName()
        {
            var file = new HavokAnimationFile { Skeletons = [], Bindings = [Binding([0], 1, 2)] };

            Assert.Equal(["Idle"], HavokClipBuilder.BuildAll(file, "Idle").Select(c => c.Name));
        }

        [Fact]
        public void BuildAll_SeveralBindings_GetNumericSuffixes()
        {
            var file = new HavokAnimationFile { Skeletons = [], Bindings = [Binding([0], 1, 2), Binding([0], 1, 2)] };

            Assert.Equal(["Idle_0", "Idle_1"], HavokClipBuilder.BuildAll(file, "Idle").Select(c => c.Name));
        }

        [Fact]
        public void DropTracksBeyondBone_TracksPastTheBoneCount_AreRemovedAndCounted()
        {
            var clip = HavokTestData.Clip("c", 1f,
                new HavokClipTrack(0, [HavokTransform.Identity]),
                new HavokClipTrack(94, [HavokTransform.Identity]),
                new HavokClipTrack(5, [HavokTransform.Identity]));

            var trimmed = HavokClipBuilder.DropTracksBeyondBone(clip, boneCount: 6, out int dropped);

            Assert.Equal(1, dropped);
            Assert.Equal([0, 5], trimmed.Tracks.Select(t => t.BoneIndex));
        }

        [Fact]
        public void DropTracksBeyondBone_NothingOutOfRange_ReturnsTheSameClip()
        {
            var clip = HavokTestData.Clip("c", 1f, new HavokClipTrack(0, [HavokTransform.Identity]));

            var same = HavokClipBuilder.DropTracksBeyondBone(clip, 3, out int dropped);

            Assert.Same(clip, same);
            Assert.Equal(0, dropped);
        }

        private static HavokAnimationBinding Binding(short[] trackToBone, int tracks, int frames, Func<int, Quaternion>? rotation = null,
            HavokBlendHint blendHint = HavokBlendHint.Normal, IReadOnlyList<HavokAnnotationTrack>? annotations = null)
        {
            var transforms = new List<HavokTransform>();
            for (int f = 0; f < frames; f++)
                for (int t = 0; t < tracks; t++)
                    transforms.Add(new HavokTransform(new Vector3(f, t, 0), rotation?.Invoke(f) ?? Quaternion.Identity, Vector3.One));

            return new HavokAnimationBinding
            {
                OriginalSkeletonName = "Root",
                TransformTrackToBoneIndices = trackToBone,
                BlendHint = blendHint,
                Animation = new HavokInterleavedAnimation
                {
                    ClassName = "hkaInterleavedUncompressedAnimation",
                    Duration = frames - 1,
                    TransformTrackCount = tracks,
                    FrameCount = frames,
                    FrameDuration = 1f,
                    AnnotationTracks = annotations ?? [],
                    Transforms = transforms,
                },
            };
        }
    }

    public class HavokGltfWriterTests
    {
        private const float Tolerance = 1e-5f;

        [Fact]
        public void WriteClipPackGlb_Skeleton_BecomesANodeHierarchyWithConvertedRestPose()
        {
            using var dir = new ArchiveTempDir();
            string path = Path.Combine(dir.Path, "pack.glb");

            HavokGltfWriter.WriteClipPackGlb(HavokTestData.TwoBoneSkeleton(), [], path);

            var model = ModelRoot.Load(path);
            var root = model.LogicalNodes.Single(n => n.Name == "Root");
            var spine = model.LogicalNodes.Single(n => n.Name == "Spine");
            Assert.Same(root, spine.VisualParent);
            AssertVector(new Vector3(0f, 1f, 0f), root.LocalTransform.Translation);
            AssertVector(new Vector3(0f, 0f, -0.5f), spine.LocalTransform.Translation);
            Assert.Equal(HavokTestData.QuarterTurnAboutZ.W, spine.LocalTransform.Rotation.W, Tolerance);
            Assert.Equal(HavokTestData.QuarterTurnAboutZ.Z, spine.LocalTransform.Rotation.Y, Tolerance);
            AssertVector(new Vector3(2f, 4f, 3f), spine.LocalTransform.Scale);
        }

        [Fact]
        public void WriteClipPackGlb_AnimatedTranslation_IsKeyedPerFrameInGltfSpace()
        {
            var skeleton = HavokTestData.TwoBoneSkeleton();
            var frames = new[]
            {
                new HavokTransform(new Vector3(0, 5, 0), HavokTestData.QuarterTurnAboutZ, new Vector3(2, 3, 4)),
                new HavokTransform(new Vector3(10, 5, 0), HavokTestData.QuarterTurnAboutZ, new Vector3(2, 3, 4)),
                new HavokTransform(new Vector3(20, 5, 5), HavokTestData.QuarterTurnAboutZ, new Vector3(2, 3, 4)),
            };
            var clip = HavokTestData.Clip("Idle", 0.5f, new HavokClipTrack(1, frames));

            var model = WriteAndLoad(skeleton, clip);

            var channel = model.LogicalAnimations.Single().FindTranslationChannel(model.LogicalNodes.Single(n => n.Name == "Spine"));
            var keys = channel.GetTranslationSampler().GetLinearKeys().ToArray();
            Assert.Equal([0f, 0.5f, 1f], keys.Select(k => k.Key));
            AssertVector(new Vector3(0f, 0f, -0.5f), keys[0].Value);
            AssertVector(new Vector3(1f, 0f, -0.5f), keys[1].Value);
            AssertVector(new Vector3(2f, 0.5f, -0.5f), keys[2].Value);
        }

        [Fact]
        public void WriteClipPackGlb_ChannelsEqualToTheRestPose_AreOmitted()
        {
            var rest = new HavokTransform(new Vector3(0, 5, 0), new Quaternion(0f, 0f, 1f, 0f), new Vector3(2, 3, 4));
            var skeleton = HavokTestData.Skeleton(("Root", -1, HavokTransform.Identity), ("Spine", 0, rest));
            var frames = new[] { rest, rest with { Translation = new Vector3(9, 9, 9) }, rest };
            var clip = HavokTestData.Clip("Idle", 1f, new HavokClipTrack(1, frames));

            var model = WriteAndLoad(skeleton, clip);

            var spine = model.LogicalNodes.Single(n => n.Name == "Spine");
            var animation = model.LogicalAnimations.Single();
            Assert.NotNull(animation.FindTranslationChannel(spine));
            Assert.Null(animation.FindRotationChannel(spine));
            Assert.Null(animation.FindScaleChannel(spine));
        }

        [Fact]
        public void WriteClipPackGlb_ConstantChannelAwayFromRest_KeepsOnlyFirstAndLastKey()
        {
            var skeleton = HavokTestData.TwoBoneSkeleton();
            var moved = HavokTestData.At(new Vector3(7, 7, 7));
            var clip = HavokTestData.Clip("Pose", 0.25f, new HavokClipTrack(0, [moved, moved, moved, moved, moved]));

            var model = WriteAndLoad(skeleton, clip);

            var keys = model.LogicalAnimations.Single().FindTranslationChannel(model.LogicalNodes.Single(n => n.Name == "Root"))
                .GetTranslationSampler().GetLinearKeys().ToArray();
            Assert.Equal([0f, 1f], keys.Select(k => k.Key));
        }

        [Fact]
        public void WriteClipPackGlb_ClipExtras_CarryLoopCycleAndAnnotations()
        {
            var skeleton = HavokTestData.TwoBoneSkeleton();
            var clip = HavokTestData.Clip("Fire", 1f, new HavokClipTrack(0, [HavokTestData.At(Vector3.Zero), HavokTestData.At(Vector3.One)])) with
            {
                Annotations = [new HavokAnnotation(0.5f, "Shot")],
            };

            var model = WriteAndLoad(skeleton, clip);

            string extras = model.LogicalAnimations.Single().Extras.ToJsonString();
            Assert.Contains("\"cycle\":\"loop\"", extras, StringComparison.Ordinal);
            Assert.Contains("\"text\":\"Shot\"", extras, StringComparison.Ordinal);
            Assert.Contains("\"t\":0.5", extras, StringComparison.Ordinal);
        }

        [Fact]
        public void WriteClipPackGlb_SeveralClips_BecomeNamedAnimationsInOrder()
        {
            var skeleton = HavokTestData.TwoBoneSkeleton();
            var track = new HavokClipTrack(0, [HavokTestData.At(Vector3.Zero), HavokTestData.At(Vector3.One)]);

            var model = WriteAndLoad(skeleton, HavokTestData.Clip("A", 1f, track), HavokTestData.Clip("B", 1f, track));

            Assert.Equal(["A", "B"], model.LogicalAnimations.Select(a => a.Name));
        }

        [Fact]
        public void WriteClipPackGlb_DuplicateBoneNames_ThrowsDuplicateBoneName()
        {
            var skeleton = HavokTestData.Skeleton(("Bone", -1, HavokTransform.Identity), ("Bone", 0, HavokTransform.Identity));

            Assert.Throws<HavokDuplicateBoneNameException>(() => HavokGltfWriter.WriteClipPackGlb(skeleton, [], "unused.glb"));
        }

        [Fact]
        public void WriteClipPackGlb_ParentAfterChild_ThrowsFormatException()
        {
            var skeleton = HavokTestData.Skeleton(("A", 1, HavokTransform.Identity), ("B", -1, HavokTransform.Identity));

            Assert.Throws<HavokFormatException>(() => HavokGltfWriter.WriteClipPackGlb(skeleton, [], "unused.glb"));
        }

        [Fact]
        public void RequireWritable_TrackDrivingABoneBeyondTheSkeleton_Throws()
        {
            var clip = HavokTestData.Clip("c", 1f, new HavokClipTrack(2, [HavokTransform.Identity, HavokTransform.Identity]));

            Assert.Throws<HavokFormatException>(() => HavokGltfWriter.RequireWritable(HavokTestData.TwoBoneSkeleton(), clip));
        }

        [Fact]
        public void RequireWritable_TwoTracksForOneBone_Throws()
        {
            var track = new HavokClipTrack(1, [HavokTransform.Identity, HavokTransform.Identity]);
            var clip = HavokTestData.Clip("c", 1f, track, track);

            Assert.Throws<HavokFormatException>(() => HavokGltfWriter.RequireWritable(HavokTestData.TwoBoneSkeleton(), clip));
        }

        [Fact]
        public void RequireWritable_TrackWithAnotherFrameCountThanTheClip_Throws()
        {
            var clip = HavokTestData.Clip("c", 1f, new HavokClipTrack(0, [HavokTransform.Identity, HavokTransform.Identity])) with { FrameCount = 5 };

            Assert.Throws<HavokFormatException>(() => HavokGltfWriter.RequireWritable(HavokTestData.TwoBoneSkeleton(), clip));
        }

        [Theory]
        [InlineData(float.NaN)]
        [InlineData(float.PositiveInfinity)]
        [InlineData(0f)]
        [InlineData(-1f)]
        public void RequireWritable_UnusableFrameDurationForSeveralFrames_Throws(float frameDuration)
        {
            var clip = HavokTestData.Clip("c", frameDuration, new HavokClipTrack(0, [HavokTransform.Identity, HavokTransform.Identity]));

            Assert.Throws<HavokFormatException>(() => HavokGltfWriter.RequireWritable(HavokTestData.TwoBoneSkeleton(), clip));
        }

        [Fact]
        public void RequireWritable_SingleFrameClipWithZeroFrameDuration_IsAccepted()
        {
            var clip = HavokTestData.Clip("pose", 0f, new HavokClipTrack(0, [HavokTransform.Identity]));

            HavokGltfWriter.RequireWritable(HavokTestData.TwoBoneSkeleton(), clip);
        }

        private static ModelRoot WriteAndLoad(HavokSkeleton skeleton, params HavokClip[] clips)
        {
            using var dir = new ArchiveTempDir();
            string path = Path.Combine(dir.Path, "pack.glb");
            HavokGltfWriter.WriteClipPackGlb(skeleton, clips, path);
            return ModelRoot.Load(path);
        }

        private static void AssertVector(Vector3 expected, Vector3 actual)
        {
            Assert.Equal(expected.X, actual.X, Tolerance);
            Assert.Equal(expected.Y, actual.Y, Tolerance);
            Assert.Equal(expected.Z, actual.Z, Tolerance);
        }
    }
}
