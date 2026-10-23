using System.Numerics;
using System.Text.Json;
using AssetConverter.Havok;
using AssetConverter.Pipeline;
using InstallerTests.Archives;
using SharpGLTF.Schema2;
using Xunit;

namespace InstallerTests.Havok
{
    /// <summary>A creature folder under a fake Meshes/Actors staging tree plus the matching output tree.</summary>
    internal sealed class CreatureFixture : IDisposable
    {
        private static readonly HkBone[] RootBones =
        [
            new("Root", -1, new Vector3(0, 0, 10), Quaternion.Identity, Vector3.One),
            new("Spine", 0, new Vector3(0, 5, 0), Quaternion.Identity, Vector3.One),
            new("Head", 1, new Vector3(1, 2, 3), Quaternion.Identity, Vector3.One),
        ];

        private static readonly HkBone[] RagdollBones = [new("Ragdoll_Root", -1, Vector3.Zero, Quaternion.Identity, Vector3.One)];

        private readonly ArchiveTempDir _dir = new();

        public string Source => Path.Combine(_dir.Path, "staging");

        public string Output => Path.Combine(_dir.Path, "assets");

        public string CreatureSource => Path.Combine(Source, "Dog");

        public string CreatureOutput => Path.Combine(Output, "Dog", "CharacterAssets");

        public CreatureFixture WithSkeleton(bool ragdollFirst = true)
        {
            var skeletons = ragdollFirst
                ? new[] { ("Ragdoll", RagdollBones), ("Root", RootBones) }
                : [("Root", RootBones)];
            WriteSource("CharacterAssets/skeleton.hkx", HkaFile.Build(skeletons, []));
            return this;
        }

        public CreatureFixture WithRenderSkeleton()
        {
            Directory.CreateDirectory(CreatureOutput);
            File.WriteAllBytes(Path.Combine(CreatureOutput, "skeleton.glb"), [1]);
            return this;
        }

        public CreatureFixture WithClip(string relativePath, string skeletonName = "Root", short[]? trackToBone = null, int frames = 3)
        {
            var map = trackToBone ?? [0, 1, 2];
            var animation = new HkInterleavedSpec(Duration: frames - 1, Tracks: map.Length, Frames: frames);
            WriteSource("Animations/" + relativePath, HkaFile.Build([], [new HkBindingSpec(skeletonName, map, 0, animation)]));
            return this;
        }

        public CreatureFixture WithSourceFile(string relativePath, byte[] bytes)
        {
            WriteSource(relativePath, bytes);
            return this;
        }

        public HavokSkeletonUnit Unit() => Assert.Single(HavokSkeletonUnits.Discover(Source, Output));

        public HavokCreatureReport Convert(int dop = 2, MemoryBudget? memory = null, CancellationToken ct = default) => HavokCreatureConverter.Convert(Unit(), dop, memory ?? MemoryBudget.Unlimited(), ct);

        public void Dispose() => _dir.Dispose();

        private void WriteSource(string relativePath, byte[] bytes)
        {
            string full = Path.Combine(CreatureSource, relativePath);
            Directory.CreateDirectory(Path.GetDirectoryName(full)!);
            File.WriteAllBytes(full, bytes);
        }
    }

    public class HavokCreatureConverterTests
    {
        private static CancellationToken Ct => TestContext.Current.CancellationToken;

        [Fact]
        public void Convert_ClipsInRootAndSubfolders_WritesOnePackPerFolderAndAnIndex()
        {
            using var fx = new CreatureFixture().WithSkeleton().WithRenderSkeleton()
                .WithClip("Idle.hkx").WithClip("Weapon/Fire.hkx").WithClip("Weapon/Reload.hkx");

            var report = fx.Convert(ct: Ct);

            Assert.Equal(3, report.ClipsOk);
            Assert.Equal(0, report.ClipsFailed);
            Assert.Equal(2, report.Packs);
            Assert.Equal(3, report.SkeletonBones);
            using var index = JsonDocument.Parse(File.ReadAllText(Path.Combine(fx.CreatureOutput, "skeleton.anims.json")));
            Assert.Equal("skeleton.glb", index.RootElement.GetProperty("skeleton").GetString());
            var packs = index.RootElement.GetProperty("packs").EnumerateArray().ToList();
            Assert.Equal(["root.anims.glb", "Weapon.anims.glb"], packs.Select(p => p.GetProperty("file").GetString()));
            Assert.Equal(["Weapon/Fire", "Weapon/Reload"], packs[1].GetProperty("clips").EnumerateArray().Select(c => c.GetString()));
        }

        [Fact]
        public async Task Convert_WaitsForTheBudgetBeforeDecodingAndReleasesThePackReservationAfterwards()
        {
            using var fx = new CreatureFixture().WithSkeleton().WithRenderSkeleton().WithClip("Idle.hkx").WithClip("Walk.hkx");
            var budget = new MemoryBudget(MemoryEstimates.HavokClipPackBytes(1L << 20));
            var other = await budget.ReserveAsync(budget.Capacity, Ct);

            var run = Task.Run(() => fx.Convert(memory: budget, ct: Ct), Ct);
            Assert.True(SpinWait.SpinUntil(() => budget.WaitingCount > 0, TimeSpan.FromSeconds(30)));
            Assert.False(run.IsCompleted);
            other.Dispose();
            var report = await run.WaitAsync(TimeSpan.FromSeconds(30), Ct);

            Assert.Equal(2, report.ClipsOk);
            Assert.Equal(0, budget.ReservedBytes);
        }

        [Fact]
        public async Task Convert_PackThatFillsTheBudgetItselfIsConvertedWithoutItsClipsReservingAgain()
        {
            using var fx = new CreatureFixture().WithSkeleton().WithRenderSkeleton().WithClip("Idle.hkx").WithClip("Walk.hkx").WithClip("Weapon/Fire.hkx");
            long packBytes = Directory.EnumerateFiles(fx.CreatureSource, "*.hkx", SearchOption.AllDirectories)
                .Where(path => !path.EndsWith("skeleton.hkx", StringComparison.OrdinalIgnoreCase) && !path.Contains("Weapon"))
                .Sum(path => new FileInfo(path).Length);
            var budget = new MemoryBudget(MemoryEstimates.HavokClipPackBytes(packBytes));

            var report = await Task.Run(() => fx.Convert(memory: budget, ct: Ct), Ct).WaitAsync(TimeSpan.FromSeconds(30), Ct);

            Assert.Equal(3, report.ClipsOk);
            Assert.Equal(2, report.Packs);
            Assert.Equal(0, budget.ReservedBytes);
        }

        [Fact]
        public async Task Convert_CancelledWhileWaitingForTheBudgetEndsWithACancellation()
        {
            using var fx = new CreatureFixture().WithSkeleton().WithRenderSkeleton().WithClip("Idle.hkx");
            var budget = new MemoryBudget(MemoryEstimates.HavokClipPackBytes(1L << 20));
            using var other = await budget.ReserveAsync(budget.Capacity, Ct);
            using var cancel = CancellationTokenSource.CreateLinkedTokenSource(Ct);

            var run = Task.Run(() => fx.Convert(memory: budget, ct: cancel.Token), CancellationToken.None);
            Assert.True(SpinWait.SpinUntil(() => budget.WaitingCount > 0, TimeSpan.FromSeconds(30)));
            await cancel.CancelAsync();

            await Assert.ThrowsAnyAsync<OperationCanceledException>(() => run.WaitAsync(TimeSpan.FromSeconds(30), Ct));
            Assert.Equal(budget.Capacity, budget.ReservedBytes);
        }

        [Fact]
        public void Convert_Pack_ContainsTheClipsAsAnimationsOnTheSelectedSkeleton()
        {
            using var fx = new CreatureFixture().WithSkeleton().WithRenderSkeleton().WithClip("Idle.hkx").WithClip("Walk.hkx");

            fx.Convert(ct: Ct);

            var model = ModelRoot.Load(Path.Combine(fx.CreatureOutput, "root.anims.glb"));
            Assert.Equal(["Idle", "Walk"], model.LogicalAnimations.Select(a => a.Name));
            Assert.Equal(["Root", "Spine", "Head"], model.LogicalNodes.Select(n => n.Name));
        }

        [Fact]
        public void Convert_FileWithSeveralSkeletons_UsesTheOneTheClipsName()
        {
            using var fx = new CreatureFixture().WithSkeleton(ragdollFirst: true).WithRenderSkeleton().WithClip("Idle.hkx");

            var report = fx.Convert(ct: Ct);

            Assert.Equal(3, report.SkeletonBones);
        }

        [Fact]
        public void Convert_ClipBoundToAnotherSkeleton_IsReportedAndTheRestOfThePackSurvives()
        {
            using var fx = new CreatureFixture().WithSkeleton().WithRenderSkeleton()
                .WithClip("Good.hkx").WithClip("Zebra.hkx", skeletonName: "Human");

            var report = fx.Convert(ct: Ct);

            Assert.Equal(1, report.ClipsOk);
            Assert.Equal(1, report.ClipsFailed);
            Assert.Contains("HavokSkeletonMismatchException", Assert.Single(report.Errors), StringComparison.Ordinal);
        }

        [Fact]
        public void Convert_CorruptClipFile_IsReportedWithoutThrowing()
        {
            using var fx = new CreatureFixture().WithSkeleton().WithRenderSkeleton()
                .WithClip("Good.hkx").WithSourceFile("Animations/Broken.hkx", new byte[200]);

            var report = fx.Convert(ct: Ct);

            Assert.Equal(1, report.ClipsOk);
            Assert.Contains("Broken", Assert.Single(report.Errors), StringComparison.Ordinal);
        }

        [Fact]
        public void Convert_HkxWithoutAnimations_IsCountedAsNonAnimationNotAsFailure()
        {
            using var fx = new CreatureFixture().WithSkeleton().WithRenderSkeleton()
                .WithClip("Good.hkx").WithSourceFile("Animations/Behavior.hkx", HkaFile.BuildWithoutAnimationContainer());

            var report = fx.Convert(ct: Ct);

            Assert.Equal(1, report.NonAnimationFiles);
            Assert.Equal(0, report.ClipsFailed);
            Assert.Empty(report.Errors);
        }

        [Fact]
        public void Convert_ClipWithTracksForBonesBeyondTheSkeleton_DropsThemWithAWarning()
        {
            using var fx = new CreatureFixture().WithSkeleton().WithRenderSkeleton()
                .WithClip("Human.hkx", trackToBone: [0, 1, 2, 50]);

            var report = fx.Convert(ct: Ct);

            Assert.Equal(1, report.TracksDropped);
            Assert.Contains("dropped 1 track", Assert.Single(report.Warnings), StringComparison.Ordinal);
            Assert.Equal(1, report.ClipsOk);
        }

        [Fact]
        public void Convert_UnitWithoutRenderSkeleton_ThrowsRenderSkeletonMissing()
        {
            using var fx = new CreatureFixture().WithSkeleton().WithClip("Idle.hkx");

            Assert.Throws<HavokRenderSkeletonMissingException>(() => fx.Convert(ct: Ct));
        }

        [Fact]
        public void Convert_AnimationsSubfolderNamedRoot_ThrowsPackNameCollision()
        {
            using var fx = new CreatureFixture().WithSkeleton().WithRenderSkeleton().WithClip("Root/Idle.hkx");

            Assert.Throws<HavokPackNameCollisionException>(() => fx.Convert(ct: Ct));
        }

        [Fact]
        public void Convert_SeveralSkeletonsAndNoClipToNameTheRightOne_ThrowsAmbiguousSkeleton()
        {
            using var fx = new CreatureFixture().WithSkeleton(ragdollFirst: true).WithRenderSkeleton();

            Assert.Throws<HavokAmbiguousSkeletonException>(() => fx.Convert(ct: Ct));
        }

        [Fact]
        public void Convert_SkeletonFileWithoutSkeletons_ThrowsFormatException()
        {
            using var fx = new CreatureFixture().WithRenderSkeleton()
                .WithSourceFile("CharacterAssets/skeleton.hkx", HkaFile.BuildWithoutAnimationContainer());

            Assert.Throws<HavokFormatException>(() => fx.Convert(ct: Ct));
        }

        [Fact]
        public void Convert_StaleOutputsFromAnEarlierRun_AreDeletedFirst()
        {
            using var fx = new CreatureFixture().WithSkeleton(ragdollFirst: false).WithRenderSkeleton();
            string stalePack = Path.Combine(fx.CreatureOutput, "old.anims.glb");
            string legacy = Path.Combine(fx.CreatureOutput, "skeleton.skel.glb");
            File.WriteAllBytes(stalePack, [1]);
            File.WriteAllBytes(legacy, [1]);

            fx.Convert(ct: Ct);

            Assert.False(File.Exists(stalePack));
            Assert.False(File.Exists(legacy));
        }

        [Fact]
        public void Convert_SameInputTwiceWithDifferentParallelism_ProducesIdenticalPacks()
        {
            using var fx = new CreatureFixture().WithSkeleton().WithRenderSkeleton();
            foreach (string name in Enumerable.Range(0, 12).Select(i => $"Clip{i:D2}.hkx"))
                fx.WithClip(name, frames: 2 + name.Length % 3);
            fx.Convert(dop: 1, ct: Ct);
            byte[] sequential = File.ReadAllBytes(Path.Combine(fx.CreatureOutput, "root.anims.glb"));

            fx.Convert(dop: 8, ct: Ct);

            Assert.Equal(sequential, File.ReadAllBytes(Path.Combine(fx.CreatureOutput, "root.anims.glb")));
        }

        [Fact]
        public void Convert_TokenAlreadyCancelled_ThrowsOperationCanceled()
        {
            using var fx = new CreatureFixture().WithSkeleton().WithRenderSkeleton().WithClip("Idle.hkx");
            using var cts = new CancellationTokenSource();
            cts.Cancel();

            Assert.ThrowsAny<OperationCanceledException>(() => fx.Convert(ct: cts.Token));
        }
    }

    // Unit table of the source files.
    public class HavokSkeletonUnitsTests
    {
        [Fact]
        public void Discover_MissingSourceFolder_ReturnsNoUnits()
        {
            Assert.Empty(HavokSkeletonUnits.Discover(Path.Combine(Path.GetTempPath(), "of4-missing-" + Guid.NewGuid().ToString("N")), "out"));
        }

        [Fact]
        public void Discover_SkeletonInCharacterAssets_BecomesAUnitWithMirroredOutputPaths()
        {
            using var dir = new ArchiveTempDir();
            Touch(dir, "src/Dog/CharacterAssets/skeleton.hkx");
            string source = Path.Combine(dir.Path, "src");
            string output = Path.Combine(dir.Path, "out");

            var unit = Assert.Single(HavokSkeletonUnits.Discover(source, output));

            Assert.Equal("Dog", unit.Label);
            Assert.Equal(Path.Combine(source, "Dog", "CharacterAssets", "skeleton.hkx"), unit.SkeletonHkxPath);
            Assert.Equal(Path.Combine(source, "Dog", "Animations"), unit.AnimationsDir);
            Assert.Equal(Path.Combine(output, "Dog", "CharacterAssets"), unit.OutputDir);
            Assert.Equal(Path.Combine(output, "Dog", "CharacterAssets", "skeleton.glb"), unit.RenderSkeletonPath);
        }

        [Fact]
        public void Discover_NestedUnit_IsSeparateFromItsParent()
        {
            using var dir = new ArchiveTempDir();
            Touch(dir, "src/Character/CharacterAssets/skeleton.hkx");
            Touch(dir, "src/Character/_1stPerson/CharacterAssets/skeleton.hkx");

            var units = HavokSkeletonUnits.Discover(Path.Combine(dir.Path, "src"), Path.Combine(dir.Path, "out"));

            Assert.Equal(["Character", "Character/_1stPerson"], units.Select(u => u.Label));
        }

        [Fact]
        public void Discover_SkeletonFileOutsideCharacterAssets_IsIgnored()
        {
            using var dir = new ArchiveTempDir();
            Touch(dir, "src/Furniture/skeleton.hkx");
            Touch(dir, "src/Props/Misc/skeleton.hkx");

            Assert.Empty(HavokSkeletonUnits.Discover(Path.Combine(dir.Path, "src"), Path.Combine(dir.Path, "out")));
        }

        [Fact]
        public void Discover_TurretAnimationFoldersAndDogmeatClips_UseTheExplicitSkeletonTable()
        {
            using var dir = new ArchiveTempDir();
            Directory.CreateDirectory(Path.Combine(dir.Path, "src", "Turret", "Animations", "Mounted"));
            Directory.CreateDirectory(Path.Combine(dir.Path, "src", "ViciousDog", "Animations"));
            string source = Path.Combine(dir.Path, "src");
            string output = Path.Combine(dir.Path, "out");

            var units = HavokSkeletonUnits.Discover(source, output);

            Assert.Equal(["Turret/Mounted", "ViciousDog"], units.Select(u => u.Label));
            Assert.Equal(Path.Combine(source, "Turret", "CharacterAssets", "TurretMountedSkeleton.hkx"), Path.GetFullPath(units[0].SkeletonHkxPath));
            Assert.Equal(Path.Combine(output, "Turret", "CharacterAssets", "TurretMountedSkeleton.glb"), Path.GetFullPath(units[0].RenderSkeletonPath));
            Assert.Equal(Path.Combine(source, "Dogmeat", "CharacterAssets", "skeleton.hkx"), Path.GetFullPath(units[1].SkeletonHkxPath));
            Assert.Equal(Path.Combine(output, "Dogmeat", "CharacterAssets", "skeleton.glb"), Path.GetFullPath(units[1].RenderSkeletonPath));
        }

        [Fact]
        public void Discover_ExplicitTableEntryWithoutItsAnimationsFolder_IsSkipped()
        {
            using var dir = new ArchiveTempDir();
            Directory.CreateDirectory(Path.Combine(dir.Path, "src", "Turret", "Animations", "Standing"));

            var units = HavokSkeletonUnits.Discover(Path.Combine(dir.Path, "src"), Path.Combine(dir.Path, "out"));

            Assert.Equal(["Turret/Standing"], units.Select(u => u.Label));
        }

        [Fact]
        public void Discover_Units_AreOrderedByLabelIgnoringCase()
        {
            using var dir = new ArchiveTempDir();
            Touch(dir, "src/zebra/CharacterAssets/skeleton.hkx");
            Touch(dir, "src/Alpha/CharacterAssets/skeleton.hkx");
            Touch(dir, "src/mole/CharacterAssets/skeleton.hkx");

            var units = HavokSkeletonUnits.Discover(Path.Combine(dir.Path, "src"), Path.Combine(dir.Path, "out"));

            Assert.Equal(["Alpha", "mole", "zebra"], units.Select(u => u.Label));
        }

        [Fact]
        public void RenderSkeletonSource_IsTheNifAtTheMirroredPathOfTheRenderSkeletonGlb()
        {
            using var dir = new ArchiveTempDir();
            Touch(dir, "src/Dog/CharacterAssets/skeleton.hkx");
            Directory.CreateDirectory(Path.Combine(dir.Path, "src", "Turret", "Animations", "Mounted"));
            Directory.CreateDirectory(Path.Combine(dir.Path, "src", "ViciousDog", "Animations"));
            string source = Path.Combine(dir.Path, "src");
            string output = Path.Combine(dir.Path, "out");

            var sources = HavokSkeletonUnits.Discover(source, output)
                .Select(unit => HavokSkeletonUnits.RenderSkeletonSource(unit, source, output))
                .ToList();

            Assert.Equal(
                [
                    Path.Combine(source, "Dog", "CharacterAssets", "skeleton.nif"),
                    Path.Combine(source, "Turret", "CharacterAssets", "TurretMountedSkeleton.nif"),
                    Path.Combine(source, "Dogmeat", "CharacterAssets", "skeleton.nif"),
                ],
                sources.Select(Path.GetFullPath));
        }

        private static void Touch(ArchiveTempDir dir, string relativePath)
        {
            string full = Path.Combine(dir.Path, relativePath);
            Directory.CreateDirectory(Path.GetDirectoryName(full)!);
            File.WriteAllBytes(full, [0]);
        }
    }
}
