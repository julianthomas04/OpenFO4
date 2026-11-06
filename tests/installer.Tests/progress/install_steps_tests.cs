using AssetConverter.Pipeline;
using AssetConverter.Progress;
using Xunit;

namespace InstallerTests.Progress
{
    public sealed class InstallStepsTests
    {
        [Fact]
        public void EveryDeclaredStepBelongsToExactlyOneLane()
        {
            var laneSteps = InstallSteps.Lanes.SelectMany(lane => lane).ToList();

            Assert.Equal(InstallSteps.All.Select(s => s.Name).Order(), laneSteps.Order());
            Assert.Equal(laneSteps.Count, laneSteps.Distinct().Count());
        }

        [Fact]
        public void TheRemovalOfTheTemporaryFilesIsPlannedUnlessTheyAreKept()
        {
            Assert.Equal(InstallSteps.All, InstallSteps.Planned(keepStaging: false));
            Assert.DoesNotContain(InstallSteps.Planned(keepStaging: true), step => step.Name == InstallSteps.RemoveStaging);
            Assert.Equal(InstallSteps.All.Count - 1, InstallSteps.Planned(keepStaging: true).Count);
        }

        [Fact]
        public void TheRemovalOfTheTemporaryFilesIsTheLastStepInItsOwnLane()
        {
            Assert.Equal(InstallSteps.RemoveStaging, InstallSteps.All[^1].Name);
            Assert.Equal([InstallSteps.RemoveStaging], InstallSteps.CleanupLane);
        }

        [Fact]
        public void ConvertingTheVideosIsDeclaredWorkInALaneOfItsOwn()
        {
            Assert.Contains(InstallSteps.All, step => step.Name == InstallSteps.ConvertVideo && step.Weight > 0);
            Assert.Equal([InstallSteps.ConvertVideo], InstallSteps.VideoLane);
            Assert.Contains(InstallSteps.VideoLane, InstallSteps.Lanes);
            Assert.DoesNotContain(InstallSteps.Planned(keepStaging: true), step => step.Name != InstallSteps.ConvertVideo && step.VisibleName == InstallSteps.ConvertVideo);
        }

        [Fact]
        public void StepsWithAFixedAmountOfWorkDeclareTheirKnownTotal()
        {
            var known = InstallSteps.All.Where(s => s.KnownTotal != null).ToDictionary(s => s.Name, s => s.KnownTotal);

            Assert.Equal(new Dictionary<string, long?>
            {
                [InstallSteps.CopyGameData] = 3,
                [InstallSteps.BakePalettes] = 1,
                [InstallSteps.ScanLodWater] = 1,
                [InstallSteps.FinalizeDatabase] = 6,
            }, known);
        }

        [Fact]
        public void OnlyInstallingTheGameLacksATotalBeforeItsGroupStartsAndItIsDeclaredFromTheUnpackedFiles()
        {
            var planned = InstallSteps.Planned(keepStaging: false, unpacksPayload: true);

            var unknownFollowers = planned.Where(s => s.Group != null)
                .GroupBy(s => s.Group)
                .SelectMany(group => group.Skip(1))
                .Where(s => s.KnownTotal == null)
                .Select(s => s.Name);

            Assert.Equal([InstallSteps.InstallGame], unknownFollowers);
        }

        [Fact]
        public void TheKnownTotalsAreFinalBeforeTheirStepsBegin()
        {
            var model = new ProgressModel(InstallSteps.All);

            var pending = model.GetSnapshot().Steps.Single(s => s.Name == InstallSteps.Finalizing);

            Assert.False(pending.IsPreparing);
            Assert.Equal(InstallSteps.FinalizeDatabaseTotal, pending.Parts.Single(p => p.Name == InstallSteps.FinalizeDatabase).Total);
            Assert.All(pending.Parts, p => Assert.True(p.TotalIsFinal));
        }

        [Fact]
        public void EveryLaneHasAtLeastOneStep()
        {
            Assert.All(InstallSteps.Lanes, lane => Assert.NotEmpty(lane));
        }

        [Fact]
        public void StepNamesAreUnique()
        {
            var names = InstallSteps.All.Select(s => s.Name).ToList();

            Assert.Equal(names.Count, names.Distinct(StringComparer.OrdinalIgnoreCase).Count());
        }

        [Fact]
        public void WeightsAreTheMeasuredSecondsOfWorkSoVideosTexturesAndMeshesDominate()
        {
            var weights = InstallSteps.All.ToDictionary(s => s.Name, s => s.Weight);

            Assert.Equal(InstallSteps.ConvertVideo, weights.MaxBy(w => w.Value).Key);
            Assert.True(weights[InstallSteps.ExtractTextures] > weights[InstallSteps.ConvertMeshes]);
            Assert.True(weights[InstallSteps.ConvertMeshes] > weights[InstallSteps.ConvertAnimations]);
            Assert.True(weights[InstallSteps.ScanLodWater] < weights[InstallSteps.FinalizeDatabase]);
        }

        [Fact]
        public void InstallingTheGameIsTheFirstStepAndRunsInALaneOfItsOwn()
        {
            Assert.Equal(InstallSteps.InstallGame, InstallSteps.All[0].Name);
            Assert.Equal([InstallSteps.InstallGame], InstallSteps.GameLane);
            Assert.Contains(InstallSteps.Planned(keepStaging: true), step => step.Name == InstallSteps.InstallGame);
        }

        [Fact]
        public void InitializingAndFinalizingAreShownAsOneStepEachWhileEveryUnitKeepsItsOwnLane()
        {
            var packaged = InstallSteps.Planned(keepStaging: false, unpacksPayload: true);
            var dev = InstallSteps.Planned(keepStaging: false);

            Assert.Equal(
                [InstallSteps.UnpackPayload, InstallSteps.InstallGame, InstallSteps.CopyGameData],
                packaged.Where(unit => unit.VisibleName == InstallSteps.Initializing).Select(unit => unit.Name));
            Assert.Equal(
                [InstallSteps.InstallGame, InstallSteps.CopyGameData],
                dev.Where(unit => unit.VisibleName == InstallSteps.Initializing).Select(unit => unit.Name));
            Assert.Equal(
                [InstallSteps.BakePalettes, InstallSteps.ScanLodWater, InstallSteps.FinalizeDatabase],
                dev.Where(unit => unit.VisibleName == InstallSteps.Finalizing).Select(unit => unit.Name));
            Assert.Equal(InstallSteps.Initializing, InstallSteps.VisibleNames(packaged)[0]);
            Assert.Equal(InstallSteps.RemoveStaging, InstallSteps.VisibleNames(packaged)[^1]);
            Assert.Equal(dev.Count - 3, InstallSteps.VisibleNames(dev).Count);
            Assert.NotEqual(InstallSteps.PaletteLane, InstallSteps.LodWaterLane);
        }

        [Fact]
        public void UnpackingThePayloadIsTheFirstPlannedStepOfAPackagedInstallerOnly()
        {
            Assert.Equal(InstallSteps.UnpackPayload, InstallSteps.Planned(keepStaging: false, unpacksPayload: true)[0].Name);
            Assert.Equal(InstallSteps.All.Count + 1, InstallSteps.Planned(keepStaging: false, unpacksPayload: true).Count);
            Assert.DoesNotContain(InstallSteps.Planned(keepStaging: false), step => step.Name == InstallSteps.UnpackPayload);
            Assert.Equal([InstallSteps.UnpackPayload], InstallSteps.UnpackLane);
        }

        private static ReconversionPlan PlanReconverting(params AssetGroup[] changed)
        {
            var installed = new InstalledAssets("1.0.0", AssetGroups.CurrentRevisions.ToDictionary(r => r.Key, r => changed.Contains(r.Key) ? r.Value - 1 : r.Value));
            return ReconversionPlan.Decide(installed, AssetGroups.CurrentRevisions);
        }

        private static string[] PlannedNames(ReconversionPlan plan) => [.. InstallSteps.Planned(keepStaging: false, reconversion: plan).Select(s => s.Name)];

        public static TheoryData<string> ChangedGroupCases => new() { "shaders", "collision", "audio+video", "meshes", "textures-database", "none" };

        private static ReconversionPlan PlanReconverting(string groupIds) =>
            PlanReconverting([.. groupIds.Split('+').Where(id => id != "none").Select(id => AssetGroups.TryFromId(id, out var group) ? group : throw new ArgumentException(id))]);

        [Fact]
        public void ReconvertingEverythingPlansTodaysWork()
        {
            Assert.Equal(InstallSteps.All, InstallSteps.Planned(keepStaging: false, reconversion: ReconversionPlan.ReconvertAll));
            Assert.Equal(InstallSteps.All, InstallSteps.Planned(keepStaging: false));
            Assert.Equal(
                InstallSteps.Planned(keepStaging: true, unpacksPayload: true),
                InstallSteps.Planned(keepStaging: true, unpacksPayload: true, reconversion: ReconversionPlan.ReconvertAll));
        }

        [Fact]
        public void OnlyInstallingTheGameAndTheCleanupBelongToNoAssetGroup()
        {
            var ungrouped = InstallSteps.All.Where(step => step.Groups.Count == 0).Select(step => step.Name);

            Assert.Equal([InstallSteps.InstallGame, InstallSteps.RemoveStaging], ungrouped);
        }

        [Fact]
        public void EveryAssetGroupOwnsAtLeastOneStep()
        {
            var owned = InstallSteps.All.SelectMany(step => step.Groups).ToHashSet();

            Assert.Equal(AssetGroups.All.ToHashSet(), owned);
        }

        [Fact]
        public void StepsOfKeptGroupsAreNotPlanned()
        {
            Assert.Equal(
                [InstallSteps.InstallGame, InstallSteps.ConvertShaders, InstallSteps.RemoveStaging],
                PlannedNames(PlanReconverting(AssetGroup.Shaders)));
            Assert.Equal(
                [InstallSteps.InstallGame, InstallSteps.ConvertAudio, InstallSteps.ConvertVideo, InstallSteps.RemoveStaging],
                PlannedNames(PlanReconverting(AssetGroup.Audio, AssetGroup.Video)));
            Assert.Equal(
                [InstallSteps.InstallGame, InstallSteps.RemoveStaging],
                PlannedNames(PlanReconverting()));
        }

        [Fact]
        public void ReconvertingTheMeshesAlsoPlansTheStepsOfTheGroupsBuiltFromThem()
        {
            var names = PlannedNames(PlanReconverting(AssetGroup.Meshes));

            Assert.Contains(InstallSteps.ConvertMeshes, names);
            Assert.Contains(InstallSteps.ConvertAnimations, names);
            Assert.Contains(InstallSteps.BuildDatabase, names);
            Assert.DoesNotContain(InstallSteps.ExtractCollision, names);
            Assert.DoesNotContain(InstallSteps.ConvertShaders, names);
        }

        [Fact]
        public void TheMeshExtractionIsPlannedWhenAnyGroupThatReadsItsOutputIsConverted()
        {
            Assert.Equal(
                [InstallSteps.InstallGame, InstallSteps.ExtractMeshes, InstallSteps.ExtractCollision, InstallSteps.RemoveStaging],
                PlannedNames(PlanReconverting(AssetGroup.Collision)));
            Assert.Contains(InstallSteps.ExtractMeshes, PlannedNames(PlanReconverting(AssetGroup.Animations)));
            Assert.Contains(InstallSteps.ExtractMeshes, PlannedNames(PlanReconverting(AssetGroup.TexturesDatabase)));
            Assert.DoesNotContain(InstallSteps.ExtractMeshes, PlannedNames(PlanReconverting(AssetGroup.Video, AssetGroup.Audio, AssetGroup.Shaders, AssetGroup.Interface)));
        }

        [Theory]
        [MemberData(nameof(ChangedGroupCases))]
        public void AFilteredPlanStillPartitionsIntoLanes(string changed)
        {
            var planned = InstallSteps.Planned(keepStaging: false, unpacksPayload: true, reconversion: PlanReconverting(changed)).Select(s => s.Name).ToList();
            var lanes = InstallSteps.Lanes.Prepend(InstallSteps.UnpackLane).ToList();

            Assert.All(planned, name => Assert.Single(lanes, lane => lane.Contains(name)));
            Assert.Equal(planned.Count, planned.Distinct().Count());
        }

        [Fact]
        public void LanesWhoseStepsAreAllKeptCanBecomeEmpty()
        {
            var planned = PlannedNames(PlanReconverting(AssetGroup.Shaders)).ToHashSet();

            var empty = InstallSteps.Lanes.Where(lane => !lane.Any(planned.Contains)).ToList();

            Assert.Contains(InstallSteps.VideoLane, empty);
            Assert.Contains(InstallSteps.TailLane, empty);
            Assert.DoesNotContain(InstallSteps.ShaderLane, empty);
        }

        [Theory]
        [MemberData(nameof(ChangedGroupCases))]
        public void ARunWithKeptGroupsLeavesNoStepPendingAndCompletesEveryShownStep(string changed)
        {
            var planned = InstallSteps.Planned(keepStaging: false, unpacksPayload: true, reconversion: PlanReconverting(changed));
            var model = new ProgressModel(planned);

            foreach (var step in planned)
                model.EndPhase(model.BeginPhase(step.Name, step.KnownTotal ?? 0, step.Unit));
            var snapshot = model.GetSnapshot();

            Assert.All(snapshot.Steps, step => Assert.Equal(StepStatus.Done, step.Status));
            Assert.Equal(InstallSteps.VisibleNames(planned), snapshot.Steps.Select(s => s.Name));
            Assert.Equal(1.0, snapshot.OverallFraction);
        }

        [Fact]
        public void KeepingTheTexturesAndTheDatabaseDropsTheFinalizingStepAndInitializingCoversTheRest()
        {
            var packaged = InstallSteps.Planned(keepStaging: false, unpacksPayload: true, reconversion: PlanReconverting(AssetGroup.Shaders));

            var visible = InstallSteps.VisibleNames(packaged);

            Assert.DoesNotContain(InstallSteps.Finalizing, visible);
            Assert.Equal(
                [InstallSteps.UnpackPayload, InstallSteps.InstallGame],
                packaged.Where(unit => unit.VisibleName == InstallSteps.Initializing).Select(unit => unit.Name));
        }
    }
}
