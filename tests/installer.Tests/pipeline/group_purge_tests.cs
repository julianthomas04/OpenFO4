using AssetConverter;
using AssetConverter.Pipeline;
using Xunit;

namespace InstallerTests.Pipeline
{
    [Collection("Console redirection")]
    public sealed class GroupPurgeTests : IDisposable
    {
        private readonly TextWriter _originalOut = Console.Out;
        private readonly TempDirectory _root = new();

        public GroupPurgeTests() => Console.SetOut(new StringWriter());

        public void Dispose()
        {
            Console.SetOut(_originalOut);
            _root.Dispose();
        }

        private InstallLocations Locations => new(Path.Combine(_root.Path, "game"), Path.Combine(_root.Path, "install"));

        private static string Write(string path)
        {
            Directory.CreateDirectory(Path.GetDirectoryName(path)!);
            File.WriteAllText(path, "x");
            return path;
        }

        private string Staging(string name) => Path.Combine(Locations.StagingRoot, name);

        /// <summary>One file of every kind of output of the install, so a purge can be seen to take its own and leave the rest.</summary>
        private Dictionary<string, string> InstallWithEveryKindOfOutput() => new()
        {
            ["fonts"] = Write(Path.Combine(Locations.FontsOut, "font.ttf")),
            ["fonts stamp"] = Write(Path.Combine(Locations.FontsOut, ".stamp")),
            ["layout"] = Write(Path.Combine(Locations.UiLayoutOut, "menu.json")),
            ["pipboy"] = Write(Path.Combine(Locations.PipboyArtOut, "frames", "a.png")),
            ["audio"] = Write(Path.Combine(Locations.AudioOut, "sound", "a.flac")),
            ["audio stamp"] = Write(Path.Combine(Locations.AudioOut, ".Fallout4 - Sounds.ba2.stamp")),
            ["video"] = Write(Path.Combine(Locations.VideoOut, "intro.mp4")),
            ["video stamp"] = Write(Path.Combine(Locations.VideoOut, "intro.mp4.stamp")),
            ["glb"] = Write(Path.Combine(Locations.MeshesOut, "a", "a.glb")),
            ["clip pack"] = Write(Path.Combine(Locations.MeshesOut, "Actors", "x", "walk.anims.glb")),
            ["collision"] = Write(Path.Combine(Locations.MeshesOut, "a", "a.col.json")),
            ["collision stamp"] = Write(Path.Combine(Locations.MeshesOut, ".collision.stamp")),
            ["texture"] = Write(Path.Combine(Locations.TexturesOut, "wall", "brick_d.dds")),
            ["database"] = Write(Locations.DatabasePath),
            ["database wal"] = Write(Locations.DatabasePath + "-wal"),
            ["database shm"] = Write(Locations.DatabasePath + "-shm"),
            ["ini"] = Write(Path.Combine(Locations.DataOut, "Fallout4_Default.ini")),
            ["shaders"] = Write(Path.Combine(Locations.AssetsRoot, "shaders", "a.wgsl")),
            ["texture marker"] = Write(Staging(".Fallout4 - Textures1.ba2.extracted-direct")),
            ["mesh marker"] = Write(Staging(".Fallout4 - Meshes.ba2.extracted")),
            ["staged nif"] = Write(Path.Combine(Locations.StagedMeshes, "a.nif")),
        };

        private static AssetGroup Group(string id) => AssetGroups.TryFromId(id, out var group) ? group : throw new ArgumentException(id);

        private HashSet<string> PurgedBy(AssetGroup group)
        {
            var files = InstallWithEveryKindOfOutput();
            GroupPurge.Purge(group, Locations, ct: TestContext.Current.CancellationToken);
            return [.. files.Where(file => !File.Exists(file.Value)).Select(file => file.Key)];
        }

        [Fact]
        public void TheInterfacePurgeTakesFontsLayoutAndPipboyArtWithTheirStamps() =>
            Assert.Equal(["fonts", "fonts stamp", "layout", "pipboy"], PurgedBy(AssetGroup.Interface).Order());

        [Fact]
        public void TheAudioPurgeTakesTheFilesAndTheStampsBesideThem() =>
            Assert.Equal(["audio", "audio stamp"], PurgedBy(AssetGroup.Audio).Order());

        [Fact]
        public void TheVideoPurgeTakesTheFilesAndTheirStamps() =>
            Assert.Equal(["video", "video stamp"], PurgedBy(AssetGroup.Video).Order());

        [Fact]
        public void TheCollisionPurgeTakesTheSidecarsAndTheStampButNoGlb() =>
            Assert.Equal(["collision", "collision stamp"], PurgedBy(AssetGroup.Collision).Order());

        [Fact]
        public void TheTexturesAndDatabasePurgeTakesTheTexturesTheDatabaseFilesAndTheExtractionMarkersOnly() =>
            Assert.Equal(["database", "database shm", "database wal", "texture", "texture marker"], PurgedBy(AssetGroup.TexturesDatabase).Order());

        [Theory]
        [InlineData("shaders")]
        [InlineData("meshes")]
        [InlineData("animations")]
        public void GroupsThatPurgeInTheirOwnConversionAreNotPurgedUpFront(string id) =>
            Assert.Empty(PurgedBy(Group(id)));

        [Fact]
        public void TheCollisionStampGoesBeforeTheSidecarsSoAnInterruptedPurgeNeverLeavesAStampForAPartialSet()
        {
            var steps = GroupPurge.StepsFor(AssetGroup.Collision, Locations);

            Assert.Equal([CollisionSidecars.StampFileName, CollisionSidecars.SidecarPattern], steps.Select(step => step.Pattern));
        }

        [Fact]
        public void PurgingWhereNothingWasInstalledDoesNothing()
        {
            foreach (var group in AssetGroups.All)
                GroupPurge.Purge(group, Locations, ct: TestContext.Current.CancellationToken);

            Assert.False(Directory.Exists(Locations.InstallRoot));
        }
    }
}
