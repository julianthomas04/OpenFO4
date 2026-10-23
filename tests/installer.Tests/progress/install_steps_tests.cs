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
    }
}
