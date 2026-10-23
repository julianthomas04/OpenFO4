using System.Numerics;
using AssetConverter.Havok;
using InstallerTests.Archives;
using Xunit;

namespace InstallerTests.Havok
{
    public class HavokClipPackTests
    {
        private const int FrameCount = 4;

        [Fact]
        public void Save_WritesTheSameBytesAsTheLibraryWriterWithoutLeavingMoreThanOneBuffer()
        {
            using var dir = new ArchiveTempDir();
            var reference = BuildPack(Clips());
            Assert.True(reference.Model.LogicalBuffers.Count > 1);
            string referencePath = Path.Combine(dir.Path, "reference.glb");
            reference.Model.SaveGLB(referencePath);

            var pack = BuildPack(Clips());
            string packPath = Path.Combine(dir.Path, "pack.glb");
            pack.Save(packPath);

            Assert.Single(pack.Model.LogicalBuffers);
            Assert.Equal(File.ReadAllBytes(referencePath), File.ReadAllBytes(packPath));
        }

        [Fact]
        public void Save_CalledTwice_WritesIdenticalFiles()
        {
            using var dir = new ArchiveTempDir();
            var pack = BuildPack(Clips());
            string first = Path.Combine(dir.Path, "first.glb");
            string second = Path.Combine(dir.Path, "second.glb");

            pack.Save(first);
            pack.Save(second);

            Assert.Equal(File.ReadAllBytes(first), File.ReadAllBytes(second));
        }

        [Fact]
        public void Save_AfterAddingClipsToAnAlreadySavedPack_WritesTheSameBytesAsAPackBuiltInOneGo()
        {
            using var dir = new ArchiveTempDir();
            var clips = Clips();
            var reference = BuildPack(clips);
            string referencePath = Path.Combine(dir.Path, "reference.glb");
            reference.Model.SaveGLB(referencePath);

            var pack = BuildPack(clips.Take(1));
            pack.Save(Path.Combine(dir.Path, "partial.glb"));
            foreach (var clip in clips.Skip(1))
                pack.Add(clip);
            string packPath = Path.Combine(dir.Path, "pack.glb");
            pack.Save(packPath);

            Assert.Equal(File.ReadAllBytes(referencePath), File.ReadAllBytes(packPath));
        }

        private static HavokGltfWriter.ClipPack BuildPack(IEnumerable<HavokClip> clips)
        {
            var pack = new HavokGltfWriter.ClipPack(Skeleton());
            foreach (var clip in clips)
                pack.Add(clip);
            return pack;
        }

        private static HavokSkeleton Skeleton() => HavokTestData.Skeleton(
            ("Root", -1, HavokTransform.Identity),
            ("Spine", 0, HavokTestData.At(new Vector3(0, 5, 0))),
            ("Head", 1, HavokTestData.At(new Vector3(0, 3, 0))));

        private static HavokClip[] Clips() =>
        [
            HavokTestData.Clip("Walk", 0.1f, Track(0, 1f), Track(1, 2f), Track(2, 3f)),
            HavokTestData.Clip("Run", 0.05f, Track(1, 4f)),
            HavokTestData.Clip("Turn", 0.2f, Track(0, 5f), Track(2, 6f)),
        ];

        private static HavokClipTrack Track(int bone, float speed) => new(bone, Enumerable.Range(0, FrameCount)
            .Select(f => new HavokTransform(
                new Vector3(f * speed, bone, f),
                Quaternion.CreateFromAxisAngle(Vector3.UnitZ, f * speed * 0.1f),
                new Vector3(1f + f * 0.01f * speed)))
            .ToArray());
    }
}
