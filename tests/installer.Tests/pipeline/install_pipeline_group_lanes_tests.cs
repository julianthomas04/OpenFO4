using AssetConverter;
using AssetConverter.Pipeline;
using AssetConverter.Progress;
using Xunit;

namespace InstallerTests.Pipeline
{
    /// <summary>The lanes of asset groups: a kept group's lane ends at once without disturbing the lanes behind it, and the marker lists exactly the groups whose lanes finished.</summary>
    [Collection("Console redirection")]
    public sealed class InstallPipelineGroupLanesTests : IDisposable
    {
        private const string Version = "2.0.0";

        private readonly TextWriter _originalOut = Console.Out;
        private readonly TextWriter _originalError = Console.Error;
        private readonly TempDirectory _root = new();

        public InstallPipelineGroupLanesTests()
        {
            Console.SetOut(new StringWriter());
            Console.SetError(new StringWriter());
            InstallProgress.Start([new StepDefinition(InstallSteps.ConvertAudio), new StepDefinition(InstallSteps.ExtractInterface)]);
        }

        public void Dispose()
        {
            Console.SetOut(_originalOut);
            Console.SetError(_originalError);
            InstallProgress.Stop();
            _root.Dispose();
        }

        private static CancellationToken Ct => TestContext.Current.CancellationToken;

        private InstallLocations Locations => new(Path.Combine(_root.Path, "game"), Path.Combine(_root.Path, "install"));

        private static LaneRunner Lanes() => new(new PhaseLog(), Ct);

        private List<AssetGroup> Recorded() => AssetVersionMarker.Read(Locations.OwnershipMarker)?.Revisions.Keys.ToList() ?? [];

        [Fact]
        public async Task AKeptGroupOpensTheGatesItsLaneWouldOpenWithoutRunningItsWork()
        {
            var skeletons = new Gate("skeletons");
            bool ran = false;

            await InstallPipeline.RunGroup(Lanes(), reconverts: false, InstallSteps.MeshLane, () => { ran = true; }, opens: [skeletons]).WithTimeout();

            Assert.True(skeletons.IsOpen);
            Assert.False(ran);
        }

        [Fact]
        public async Task AConvertedGroupRunsItsWorkAndItsLaneEndsOnlyOnceItOpenedItsGates()
        {
            var skeletons = new Gate("skeletons");
            bool ran = false;

            await InstallPipeline.RunGroup(Lanes(), reconverts: true, InstallSteps.MeshLane, () => { ran = true; skeletons.Open(); }, opens: [skeletons]).WithTimeout();

            Assert.True(ran);
            Assert.True(skeletons.IsOpen);
        }

        [Fact]
        public async Task AKeptGroupsLaneWaitsForWhatItsRealLaneWouldWaitFor()
        {
            var inputs = new Gate("inputs");
            var skeletons = new Gate("skeletons");

            var lane = InstallPipeline.RunGroup(Lanes(), reconverts: false, InstallSteps.MeshLane, () => { }, after: [inputs], opens: [skeletons]);

            Assert.False(lane.IsCompleted);
            Assert.False(skeletons.IsOpen);
            inputs.Open();
            await lane.WithTimeout();
            Assert.True(skeletons.IsOpen);
        }

        [Fact]
        public async Task AKeptGroupsLaneNeverMarksItsProgressStepFailedEvenWhenTheLaneEndsInFailure()
        {
            var failure = new InvalidOperationException("an earlier lane failed");

            var thrown = await Assert.ThrowsAsync<InvalidOperationException>(() =>
                InstallPipeline.RunGroup(Lanes(), reconverts: false, [InstallSteps.ConvertAudio], () => { }, after: [Task.FromException(failure)]).WithTimeout());

            Assert.Same(failure, thrown);
            Assert.Equal(StepStatus.Pending, InstallProgress.Model!.GetSnapshot().Steps.Single(step => step.Name == InstallSteps.ConvertAudio).Status);
        }

        [Fact]
        public void AConvertedGroupIsListedInTheMarkerOnceItsWorkHasReturnedAtItsCurrentRevision()
        {
            Locations.CreateOutputDirectories();
            bool listedWhileWorking = true;
            var conversion = InstallPipeline.Converting(AssetGroup.Audio, Locations, Version, () => listedWhileWorking = Recorded().Contains(AssetGroup.Audio), Ct);

            conversion();

            Assert.False(listedWhileWorking);
            var recorded = AssetVersionMarker.Read(Locations.OwnershipMarker)!;
            Assert.Equal(Version, recorded.Version);
            Assert.Equal(AssetGroups.Revision(AssetGroup.Audio), recorded.Revisions[AssetGroup.Audio]);
        }

        [Fact]
        public void AGroupWhoseWorkFailsIsNotListed()
        {
            Locations.CreateOutputDirectories();
            var conversion = InstallPipeline.Converting(AssetGroup.Audio, Locations, Version, () => throw new InvalidOperationException("converter exploded"), Ct);

            Assert.Throws<InvalidOperationException>(conversion);

            Assert.Empty(Recorded());
        }

        [Fact]
        public async Task AFailedRunLeavesTheGroupsItKeptAndFinishedRecordedAndTheOneItDidNotFinishAbsent()
        {
            Locations.CreateOutputDirectories();
            var everythingOld = new InstalledAssets("1.0.0", AssetGroups.All.ToDictionary(group => group, group => 1));
            var audioAndVideoChanged = new Dictionary<AssetGroup, int>(AssetGroups.CurrentRevisions) { [AssetGroup.Audio] = 2, [AssetGroup.Video] = 2 };
            AssetVersionMarker.Write(Locations.OwnershipMarker, everythingOld);
            var plan = ReconversionPlan.Decide(everythingOld, audioAndVideoChanged);
            Locations.WithdrawReconverted(plan, Version);
            var lanes = Lanes();

            var audio = InstallPipeline.RunGroup(lanes, plan.Reconverts(AssetGroup.Audio), InstallSteps.AudioLane,
                InstallPipeline.Converting(AssetGroup.Audio, Locations, Version, () => { }, Ct));
            var video = InstallPipeline.RunGroup(lanes, plan.Reconverts(AssetGroup.Video), InstallSteps.VideoLane,
                InstallPipeline.Converting(AssetGroup.Video, Locations, Version, () => throw new InvalidOperationException("video failed"), Ct), after: [audio]);
            await Assert.ThrowsAsync<InvalidOperationException>(() => Task.WhenAll(audio, video).WithTimeout());

            Assert.Equal(AssetGroups.All.Where(group => group != AssetGroup.Video).Order(), Recorded().Order());
        }

        [Fact]
        public async Task AGroupWhoseRecordCannotBeWrittenFailsItsLaneInsteadOfEndingAsDone()
        {
            Locations.CreateOutputDirectories();
            File.Delete(Locations.OwnershipMarker);
            Directory.CreateDirectory(Locations.OwnershipMarker);

            var failure = await Assert.ThrowsAsync<IOException>(() => InstallPipeline.RunGroup(Lanes(), reconverts: true, InstallSteps.AudioLane,
                InstallPipeline.Converting(AssetGroup.Audio, Locations, Version, () => { }, Ct)).WithTimeout());

            Assert.Contains(Locations.OwnershipMarker, failure.Message, StringComparison.Ordinal);
        }

        [Fact]
        public void AConvertedGroupPurgesItsOutputBeforeItsConversionPlansSoAMatchingStampCannotKeepOutputThatShouldBeReplaced()
        {
            Locations.CreateOutputDirectories();
            var stamp = new ConversionStamp(1, "en", []);
            Directory.CreateDirectory(Locations.FontsOut);
            File.WriteAllText(Path.Combine(Locations.FontsOut, "font.ttf"), "old");
            stamp.WriteSidecar(Locations.FontsOut);
            Assert.Null(stamp.SidecarStaleReason(Locations.FontsOut));
            int conversions = 0;
            PendingExtraction? Plan() => stamp.SidecarStaleReason(Locations.FontsOut) is null
                ? null
                : new PendingExtraction("fonts", () =>
                {
                    conversions++;
                    Directory.CreateDirectory(Locations.FontsOut);
                    File.WriteAllText(Path.Combine(Locations.FontsOut, "font.ttf"), "new");
                    stamp.WriteSidecar(Locations.FontsOut);
                });
            var stages = new PipelineStages(Locations, MemoryBudget.Unlimited(), Ct);

            InstallPipeline.Converting(AssetGroup.Interface, Locations, Version, () => stages.ExtractInterfaceAssets([Plan]), Ct)();

            Assert.Equal(1, conversions);
            Assert.Equal("new", File.ReadAllText(Path.Combine(Locations.FontsOut, "font.ttf")));
            Assert.Contains(AssetGroup.Interface, Recorded());
        }
    }
}
