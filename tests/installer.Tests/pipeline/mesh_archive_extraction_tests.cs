using AssetConverter;
using AssetConverter.Logging;
using AssetConverter.Pipeline;
using AssetConverter.Progress;
using InstallerTests.Archives;
using Xunit;

namespace InstallerTests.Pipeline
{
    /// <summary>
    /// <see cref="PipelineStages.ExtractMeshArchives"/> with synthetic archives: the staging order, the hand-over of NIFs to the
    /// conversion queue while the archives extract, and what happens when the extraction fails. Drives the process-wide InstallProgress
    /// and prints to the console, so it runs serially with the other tests that do.
    /// </summary>
    [Collection("Console redirection")]
    public sealed class MeshArchiveExtractionTests : IDisposable
    {
        private const string MaterialsArchive = "Fallout4 - Materials.ba2";
        private const string AnimationsArchive = "Fallout4 - Animations.ba2";
        private const string MeshesArchive = "Fallout4 - Meshes.ba2";
        private const string MeshesExtraArchive = "Fallout4 - MeshesExtra.ba2";

        private static CancellationToken Ct => TestContext.Current.CancellationToken;

        private readonly TempDirectory _root = new();
        private readonly ConsoleCapture _capture = ConsoleCapture.Start(new LogBuffer(1000));
        private readonly InstallLocations _paths;

        public MeshArchiveExtractionTests()
        {
            InstallProgress.Start([new StepDefinition(InstallSteps.ExtractMeshes)]);
            _paths = new InstallLocations(Path.Combine(_root.Path, "game"), Path.Combine(_root.Path, "install"));
            Directory.CreateDirectory(_paths.GameDataDirectory);
            Directory.CreateDirectory(_paths.StagingRoot);
        }

        public void Dispose()
        {
            InstallProgress.Stop();
            _capture.Dispose();
            _root.Dispose();
        }

        private static Ba2File Entry(string name, int length, int seed) => new(name, TestBytes.Random(length, seed), Compress: seed % 2 == 0);

        private void WriteArchive(string name, params Ba2File[] files) =>
            File.WriteAllBytes(Path.Combine(_paths.GameDataDirectory, name), Ba2Builder.Gnrl(files));

        private void MarkExtracted(string marker) => File.WriteAllText(Path.Combine(_paths.StagingRoot, marker), "");

        private string Staged(string relative) => Path.Combine(_paths.StagingRoot, relative);

        private static string Skeleton(string extension) => Path.Combine("Meshes", "Actors", "Dog", "CharacterAssets", $"skeleton{extension}");

        private PipelineStages Stages(CancellationToken? ct = null) => new(_paths, MemoryBudget.Unlimited(), ct ?? Ct);

        private static StepSnapshot Step() => InstallProgress.Model!.GetSnapshot().Steps.Single(s => s.Name == InstallSteps.ExtractMeshes);

        private static List<string> Drain(NifWorkQueue queue)
        {
            var taken = new List<string>();
            while (queue.TryTake(out string path, Ct)) taken.Add(path);
            return taken;
        }

        private void WriteColdGameArchives()
        {
            WriteArchive(MaterialsArchive, Entry(@"materials\wall.bgsm", 120, 1));
            WriteArchive(AnimationsArchive, Entry(@"Meshes\Actors\Dog\CharacterAssets\skeleton.hkx", 80, 2), Entry(@"Meshes\Actors\Dog\Animations\walk.hkx", 90, 3),
                Entry(@"Meshes\Other\ignored.hkx", 90, 4));
            WriteArchive(MeshesArchive, Entry(@"Meshes\a\one.nif", 3000, 5), Entry(@"Meshes\a\notes.txt", 40, 6), Entry(@"Meshes\b\two.nif", 500, 7));
            WriteArchive(MeshesExtraArchive, Entry(@"Meshes\c\three.nif", 4000, 8), Entry(@"Meshes\Actors\Dog\CharacterAssets\skeleton.nif", 700, 9));
        }

        [Fact]
        public async Task EveryNifIsHandedOverOnceWithItsCompleteContentWhileTheArchivesStillExtract()
        {
            WriteColdGameArchives();
            var skeletons = new PriorityFiles(() => { });
            var queue = new NifWorkQueue(priorityFiles: skeletons);
            var taken = new List<(string Path, byte[] Content)>();
            var consumer = Task.Run(() =>
            {
                while (queue.TryTake(out string path, Ct)) taken.Add((path, File.ReadAllBytes(path)));
            }, Ct);

            Stages().ExtractMeshArchives(queue, skeletons, () => { });
            await consumer.WithTimeout();

            Assert.Equal(4, taken.Count);
            Assert.Equal(4, queue.ExpectedCount);
            Assert.Equal(TestBytes.Random(3000, 5), taken.Single(t => t.Path == Staged(@"Meshes\a\one.nif")).Content);
            Assert.Equal(TestBytes.Random(500, 7), taken.Single(t => t.Path == Staged(@"Meshes\b\two.nif")).Content);
            Assert.Equal(TestBytes.Random(4000, 8), taken.Single(t => t.Path == Staged(@"Meshes\c\three.nif")).Content);
            Assert.DoesNotContain(taken, t => t.Path.EndsWith(".txt", StringComparison.Ordinal) || t.Path.EndsWith(".hkx", StringComparison.Ordinal));
        }

        [Fact]
        public void TheInputsGateOpensAfterMaterialsAndHkxAreStagedAndBeforeAnyNif()
        {
            WriteColdGameArchives();
            var skeletons = new PriorityFiles(() => { });
            var queue = new NifWorkQueue(priorityFiles: skeletons);
            bool materialsStaged = false, hkxStaged = false, anyNifStaged = true;

            Stages().ExtractMeshArchives(queue, skeletons, () =>
            {
                materialsStaged = File.Exists(Staged(@"materials\wall.bgsm"));
                hkxStaged = File.Exists(Staged(Skeleton(".hkx"))) && File.Exists(Staged(@"Meshes\Actors\Dog\Animations\walk.hkx"));
                anyNifStaged = Directory.EnumerateFiles(_paths.StagingRoot, "*.nif", SearchOption.AllDirectories).Any();
            });

            Assert.True(materialsStaged);
            Assert.True(hkxStaged);
            Assert.False(anyNifStaged);
            Assert.False(File.Exists(Staged(@"Meshes\Other\ignored.hkx")));
        }

        [Theory]
        [InlineData(MeshesArchive, @"materials\extra.bgsm")]
        [InlineData(MeshesExtraArchive, @"materials\extra.bgsm")]
        [InlineData(MeshesExtraArchive, @"Meshes\props\glow.bgem")]
        public void TheInputsGateWaitsForTheLastMeshArchiveThatHoldsAMaterialFile(string holder, string material)
        {
            WriteColdGameArchives();
            WriteArchive(holder, Entry(material, 60, 10), Entry(@"Meshes\holder.nif", 300, 11));
            bool materialStaged = false, holderNifStaged = false, laterArchiveNifStaged = true;

            Stages().ExtractMeshArchives(new NifWorkQueue(), new PriorityFiles(() => { }), () =>
            {
                materialStaged = File.Exists(Staged(material));
                holderNifStaged = File.Exists(Staged(@"Meshes\holder.nif"));
                laterArchiveNifStaged = File.Exists(Staged(@"Meshes\c\three.nif"));
            });

            Assert.True(materialStaged);
            Assert.True(holderNifStaged);
            Assert.False(laterArchiveNifStaged);
        }

        [Fact]
        public void AMaterialTheMaterialsArchiveHoldsButAMeshArchiveOverridesIsStagedWithTheOverridingContentWhenTheGateOpens()
        {
            WriteColdGameArchives();
            WriteArchive(MeshesExtraArchive, Entry(@"Materials\WALL.BGSM", 77, 11), Entry(@"Meshes\c\three.nif", 4000, 8));
            byte[]? contentWhenReady = null;

            Stages().ExtractMeshArchives(new NifWorkQueue(), new PriorityFiles(() => { }), () => contentWhenReady = File.ReadAllBytes(Staged(@"materials\wall.bgsm")));

            Assert.Equal(TestBytes.Random(77, 11), contentWhenReady);
        }

        [Fact]
        public void TheRenderSkeletonIsServedFirstEvenThoughItsArchiveIsExtractedLast()
        {
            WriteColdGameArchives();
            var signals = 0;
            var skeletons = new PriorityFiles(() => signals++);
            var queue = new NifWorkQueue(priorityFiles: skeletons);

            Stages().ExtractMeshArchives(queue, skeletons, () => { });
            skeletons.Expect(queue.Expected);
            var served = Drain(queue);

            Assert.Equal(Staged(Skeleton(".nif")), served[0]);
            Assert.Equal(0, signals);
            skeletons.Converted(served[0]);
            Assert.Equal(1, signals);
        }

        [Fact]
        public void ANifInBothMeshArchivesIsWrittenAndHandedOverOnceFromTheLastArchive()
        {
            WriteArchive(MeshesArchive, Entry(@"Meshes\shared.nif", 300, 1), Entry(@"Meshes\only1.nif", 30, 2));
            WriteArchive(MeshesExtraArchive, Entry(@"meshes\SHARED.NIF", 400, 3));
            var queue = new NifWorkQueue();

            Stages().ExtractMeshArchives(queue, new PriorityFiles(() => { }), () => { });
            var served = Drain(queue);

            Assert.Equal(2, served.Count);
            Assert.Equal(2, queue.ExpectedCount);
            Assert.Equal(TestBytes.Random(400, 3), File.ReadAllBytes(Staged(@"Meshes\shared.nif")));
        }

        [Fact]
        public void TheProgressTotalIsKnownUpFrontAndEqualsTheFilesWritten()
        {
            WriteColdGameArchives();
            var queue = new NifWorkQueue();

            Stages().ExtractMeshArchives(queue, new PriorityFiles(() => { }), () => { });

            Assert.Equal(8, Step().Total);
            Assert.Equal(8, Step().Completed);
        }

        [Fact]
        public void AWarmRunQueuesNothingOpensTheInputsGateAndCompletesTheQueue()
        {
            WriteColdGameArchives();
            foreach (string marker in new[] { $".{MaterialsArchive}.extracted", $".{AnimationsArchive}.extracted-hkx", $".{MeshesArchive}.extracted", $".{MeshesExtraArchive}.extracted" })
                MarkExtracted(marker);
            var queue = new NifWorkQueue();
            bool inputsReady = false;

            Stages().ExtractMeshArchives(queue, new PriorityFiles(() => { }), () => inputsReady = true);

            Assert.True(inputsReady);
            Assert.True(queue.IsComplete);
            Assert.Empty(Drain(queue));
            Assert.Equal(0, queue.ExpectedCount);
            Assert.DoesNotContain(Directory.EnumerateFiles(_paths.StagingRoot, "*", SearchOption.AllDirectories), f => !Path.GetFileName(f).StartsWith('.'));
        }

        [Fact]
        public void AFailedEntryLeavesItsNifOutOfTheQueueButInTheAnnouncedTotalAndWithholdsTheMarker()
        {
            WriteArchive(MeshesArchive, Entry(@"Meshes\good.nif", 500, 1));
            var damaged = Ba2Builder.Gnrl([Entry(@"Meshes\bad.nif", 6000, 2), Entry(@"Meshes\fine.nif", 700, 3)], nameTable: true);
            for (int i = 24 + 2 * 36 + 4; i < 24 + 2 * 36 + 4 + 16; i++)
                damaged[i] ^= 0xFF;
            File.WriteAllBytes(Path.Combine(_paths.GameDataDirectory, MeshesExtraArchive), damaged);
            var queue = new NifWorkQueue();

            Stages().ExtractMeshArchives(queue, new PriorityFiles(() => { }), () => { });

            Assert.Equal(3, queue.ExpectedCount);
            Assert.Equal(2, Drain(queue).Count);
            Assert.True(File.Exists(Staged($".{MeshesArchive}.extracted")));
            Assert.False(File.Exists(Staged($".{MeshesExtraArchive}.extracted")));
        }

        private static byte[] NifBytes(bool withCollision, int seed) =>
            [.. TestBytes.Random(2000, seed), .. System.Text.Encoding.ASCII.GetBytes(withCollision ? "bhkPhysicsSystem" : "bhkPhysicsSyste"), .. TestBytes.Random(300, seed + 1)];

        private static Ba2File NifEntry(string name, bool withCollision, int seed, bool compress = true) => new(name, NifBytes(withCollision, seed), compress);

        private void StageNif(string relative, bool withCollision, int seed)
        {
            string path = Staged(relative);
            Directory.CreateDirectory(Path.GetDirectoryName(path)!);
            File.WriteAllBytes(path, NifBytes(withCollision, seed));
        }

        private void MarkMeshArchivesExtracted()
        {
            MarkExtracted($".{MeshesArchive}.extracted");
            MarkExtracted($".{MeshesExtraArchive}.extracted");
        }

        private static string[] Extract(PipelineStages stages)
        {
            stages.ExtractMeshArchives(new NifWorkQueue(), new PriorityFiles(() => { }), () => { });
            stages.QueueUnobservedCollisionCandidates();
            return [.. Drain(stages.CollisionWork).Order(StringComparer.OrdinalIgnoreCase)];
        }

        [Fact]
        public async Task ANifWithTheCollisionMarkerIsQueuedForTheWorkerOnceWrittenWhileTheExtractionIsStillRunning()
        {
            WriteArchive(MeshesArchive, NifEntry(@"Meshes\a\solid.nif", true, 1), NifEntry(@"Meshes\a\plain.nif", false, 2));
            WriteArchive(MeshesExtraArchive, NifEntry(@"Meshes\b\solid2.nif", true, 3));
            var stages = Stages();
            var streamed = new List<string>();
            var consumer = Task.Run(() =>
            {
                while (stages.CollisionWork.TryTake(out string path, Ct)) streamed.Add(path);
            }, Ct);

            stages.ExtractMeshArchives(new NifWorkQueue(), new PriorityFiles(() => { }), () => { });
            var deadline = DateTime.UtcNow + TimeSpan.FromSeconds(10);
            while (stages.CollisionWork.ExpectedCount < 2 && DateTime.UtcNow < deadline) await Task.Delay(5, Ct);

            Assert.False(stages.CollisionWork.IsComplete);
            Assert.Equal(2, stages.CollisionWork.ExpectedCount);
            stages.QueueUnobservedCollisionCandidates();
            await consumer.WithTimeout();
            Assert.Equal([Staged(@"Meshes\a\solid.nif"), Staged(@"Meshes\b\solid2.nif")], streamed.Order(StringComparer.OrdinalIgnoreCase));
        }

        [Fact]
        public void TheNifsSeenDuringTheExtractionAreQueuedBeforeTheOnesLeftByAnEarlierRun()
        {
            WriteArchive(MeshesArchive, NifEntry(@"Meshes\zz\new_solid.nif", true, 1));
            MarkExtracted($".{MeshesExtraArchive}.extracted");
            StageNif(@"Meshes\aa\old.nif", withCollision: true, seed: 2);
            var stages = Stages();

            stages.ExtractMeshArchives(new NifWorkQueue(), new PriorityFiles(() => { }), () => { });
            stages.QueueUnobservedCollisionCandidates();

            Assert.Equal([Staged(@"Meshes\zz\new_solid.nif"), Staged(@"Meshes\aa\old.nif")], Drain(stages.CollisionWork), StringComparer.OrdinalIgnoreCase);
        }

        [Fact]
        public void ANifQueuedDuringTheExtractionIsNotQueuedAgainWhenAnEarlierRunLeftItInAnotherCase()
        {
            StageNif(@"Meshes\a\solid.nif", withCollision: true, seed: 1);
            WriteArchive(MeshesArchive, NifEntry(@"meshes\A\SOLID.NIF", true, 2));
            MarkExtracted($".{MeshesExtraArchive}.extracted");

            var candidates = Extract(Stages());

            Assert.Single(candidates);
        }

        [Fact]
        public void AnExtractionFailureEndsTheCollisionQueueWithTheSameExceptionAndTheBacklogIsNeverQueued()
        {
            WriteArchive(MeshesArchive, NifEntry(@"Meshes\good.nif", true, 1));
            File.WriteAllBytes(Path.Combine(_paths.GameDataDirectory, MeshesExtraArchive), [1, 2, 3]);
            var stages = Stages();

            var thrown = Assert.ThrowsAny<Exception>(() => stages.ExtractMeshArchives(new NifWorkQueue(), new PriorityFiles(() => { }), () => { }));

            Assert.Same(thrown, Assert.ThrowsAny<Exception>(() => stages.CollisionWork.TryTake(out _, Ct)));
        }

        [Fact]
        public void OnlyTheNifsWrittenWithTheCollisionMarkerAreCollisionCandidatesAfterAColdExtraction()
        {
            WriteArchive(MeshesArchive, NifEntry(@"Meshes\a\solid.nif", true, 1), NifEntry(@"Meshes\a\plain.nif", false, 2, compress: false),
                NifEntry(@"Meshes\a\notes.txt", true, 3, compress: false));
            WriteArchive(MeshesExtraArchive, NifEntry(@"Meshes\b\solid2.nif", true, 4, compress: false), NifEntry(@"Meshes\b\plain2.nif", false, 5));

            var candidates = Extract(Stages());

            Assert.Equal([Staged(@"Meshes\a\solid.nif"), Staged(@"Meshes\b\solid2.nif")], candidates, StringComparer.OrdinalIgnoreCase);
        }

        [Fact]
        public void ANifThatTheLastArchiveOverwritesIsJudgedByTheContentThatWasWrittenLast()
        {
            WriteArchive(MeshesArchive, NifEntry(@"Meshes\shared.nif", true, 1));
            WriteArchive(MeshesExtraArchive, NifEntry(@"meshes\SHARED.NIF", false, 2));

            Assert.Empty(Extract(Stages()));
        }

        [Fact]
        public void EveryNifLeftByAnEarlierRunIsACandidateAndTheWorkersPrefilterDecidesIt()
        {
            WriteColdGameArchives();
            foreach (string marker in new[] { $".{MaterialsArchive}.extracted", $".{AnimationsArchive}.extracted-hkx" })
                MarkExtracted(marker);
            MarkMeshArchivesExtracted();
            StageNif(@"Meshes\old\solid.nif", withCollision: true, seed: 1);
            StageNif(@"Meshes\old\plain.nif", withCollision: false, seed: 2);

            var candidates = Extract(Stages());

            Assert.Equal([Staged(@"Meshes\old\plain.nif"), Staged(@"Meshes\old\solid.nif")], candidates);
        }

        [Fact]
        public void ANifLeftByAnEarlierRunIsListedNextToTheOnesThisRunCheckedAndANifThisRunSawWithoutTheMarkerIsNot()
        {
            WriteArchive(MeshesArchive, NifEntry(@"Meshes\early\old_plain.nif", false, 1));
            MarkExtracted($".{MeshesArchive}.extracted");
            StageNif(@"Meshes\early\old_plain.nif", withCollision: false, seed: 1);
            StageNif(@"Meshes\early\old_solid.nif", withCollision: true, seed: 2);
            WriteArchive(MeshesExtraArchive, NifEntry(@"Meshes\late\new_solid.nif", true, 3), NifEntry(@"Meshes\late\new_plain.nif", false, 4));

            var candidates = Extract(Stages());

            Assert.Equal([Staged(@"Meshes\early\old_plain.nif"), Staged(@"Meshes\early\old_solid.nif"), Staged(@"Meshes\late\new_solid.nif")], candidates);
        }

        [Fact]
        public void ANifWhoseExtractionFailedStaysACandidateSoTheWorkerStillSeesWhateverIsOnDisk()
        {
            WriteArchive(MeshesArchive, NifEntry(@"Meshes\good_plain.nif", false, 1));
            var damaged = Ba2Builder.Gnrl([NifEntry(@"Meshes\bad.nif", true, 2), NifEntry(@"Meshes\fine_plain.nif", false, 3)], nameTable: true);
            for (int i = 24 + 2 * 36 + 4; i < 24 + 2 * 36 + 4 + 16; i++)
                damaged[i] ^= 0xFF;
            File.WriteAllBytes(Path.Combine(_paths.GameDataDirectory, MeshesExtraArchive), damaged);

            var candidates = Extract(Stages());

            Assert.Equal([Staged(@"Meshes\bad.nif")], candidates, StringComparer.OrdinalIgnoreCase);
        }

        [Fact]
        public void TheQueuedPathsAreSpelledLikeTheStagedTreeWhateverCaseTheArchivesUse()
        {
            WriteArchive(MeshesArchive, Entry(@"Meshes\Actors\Dog\a.nif", 300, 1));
            WriteArchive(MeshesExtraArchive, Entry(@"meshes\actors\dog\b.nif", 300, 2), Entry(@"MESHES\ACTORS\DOG\c.nif", 300, 3));
            var queue = new NifWorkQueue();

            Stages().ExtractMeshArchives(queue, new PriorityFiles(() => { }), () => { });

            var onDisk = Directory.EnumerateFiles(_paths.StagingRoot, "*.nif", SearchOption.AllDirectories);
            Assert.Equal(onDisk.Order(StringComparer.Ordinal), Drain(queue).Order(StringComparer.Ordinal));
        }

        [Fact]
        public void ANifListedTwiceInOneArchiveIsQueuedOnceAndTheMarkerIsWritten()
        {
            WriteArchive(MeshesArchive, Entry(@"Meshes\dup.nif", 300, 1), Entry(@"meshes\DUP.NIF", 400, 2), Entry(@"Meshes\other.nif", 100, 3));
            var queue = new NifWorkQueue();

            Stages().ExtractMeshArchives(queue, new PriorityFiles(() => { }), () => { });

            Assert.Equal(2, Drain(queue).Count);
            Assert.Equal(2, queue.ExpectedCount);
            Assert.Equal(TestBytes.Random(400, 2), File.ReadAllBytes(Staged(@"Meshes\dup.nif")));
            Assert.True(File.Exists(Staged($".{MeshesArchive}.extracted")));
        }

        [Fact]
        public async Task AnExtractionFailureWakesAWaitingConsumerWithTheSameExceptionTheStageThrows()
        {
            WriteArchive(MeshesArchive, Entry(@"Meshes\good.nif", 500, 1));
            File.WriteAllBytes(Path.Combine(_paths.GameDataDirectory, MeshesExtraArchive), [1, 2, 3]);
            var queue = new NifWorkQueue();
            var consumer = Task.Run(() => Drain(queue), Ct);

            var thrown = Assert.ThrowsAny<Exception>(() => Stages().ExtractMeshArchives(queue, new PriorityFiles(() => { }), () => { }));

            var consumerError = await Assert.ThrowsAnyAsync<Exception>(() => consumer.WithTimeout());
            Assert.Same(thrown, consumerError);
        }

        [Fact]
        public async Task CancellingTheExtractionEndsTheQueueWithTheCancellation()
        {
            WriteColdGameArchives();
            using var cts = new CancellationTokenSource();
            await cts.CancelAsync();
            var queue = new NifWorkQueue();

            var thrown = Assert.ThrowsAny<OperationCanceledException>(() =>
                Stages(cts.Token).ExtractMeshArchives(queue, new PriorityFiles(() => { }), () => { }));

            Assert.Same(thrown, Assert.ThrowsAny<OperationCanceledException>(() => queue.TryTake(out _, Ct)));
        }
    }
}
