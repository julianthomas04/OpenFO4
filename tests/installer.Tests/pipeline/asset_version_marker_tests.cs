using AssetConverter.Pipeline;
using Xunit;

namespace InstallerTests.Pipeline
{
    public sealed class AssetVersionMarkerTests : IDisposable
    {
        private readonly TempDirectory _root = new();

        public void Dispose() => _root.Dispose();

        private string MarkerPath => Path.Combine(_root.Path, "marker");

        private static InstalledAssets Assets(string version, params (AssetGroup Group, int Revision)[] revisions) =>
            new(version, revisions.ToDictionary(r => r.Group, r => r.Revision));

        [Fact]
        public void TheMarkerIsTheVersionLineThenOneLinePerGroupInTableOrderWithoutAByteOrderMark()
        {
            AssetVersionMarker.Write(MarkerPath, Assets("1.22.333", (AssetGroup.TexturesDatabase, 4), (AssetGroup.Shaders, 3), (AssetGroup.Meshes, 2)));

            Assert.Equal("Asset Version 1.22.333\nshaders=3\nmeshes=2\ntextures-database=4\n"u8.ToArray(), File.ReadAllBytes(MarkerPath));
        }

        [Fact]
        public void ARecordWithoutGroupsIsJustTheVersionLine()
        {
            AssetVersionMarker.Write(MarkerPath, Assets("1.0.0"));

            Assert.Equal("Asset Version 1.0.0\n"u8.ToArray(), File.ReadAllBytes(MarkerPath));
        }

        [Fact]
        public void EveryGroupRoundTripsWithItsRevision()
        {
            var written = new InstalledAssets("0.0.0-dev", AssetGroups.All.Select((group, index) => (group, revision: index + 1)).ToDictionary(g => g.group, g => g.revision));

            AssetVersionMarker.Write(MarkerPath, written);

            var read = AssetVersionMarker.Read(MarkerPath);
            Assert.Equal("0.0.0-dev", read?.Version);
            Assert.Equal(written.Revisions.OrderBy(r => r.Key), read!.Revisions.OrderBy(r => r.Key));
        }

        [Fact]
        public void ThePendingMarkerIsEmptyAndReadsAsNoRecord()
        {
            AssetVersionMarker.WritePending(MarkerPath);

            Assert.Empty(File.ReadAllBytes(MarkerPath));
            Assert.Null(AssetVersionMarker.Read(MarkerPath));
        }

        [Fact]
        public void AMissingFileReadsAsNoRecord()
        {
            Assert.Null(AssetVersionMarker.Read(MarkerPath));
        }

        [Fact]
        public void WritingReplacesAnEarlierMarkerAndLeavesNoTemporaryFile()
        {
            AssetVersionMarker.Write(MarkerPath, Assets("1.0.0", (AssetGroup.Audio, 1)));

            AssetVersionMarker.Write(MarkerPath, Assets("2.0.0", (AssetGroup.Video, 5)));

            var read = AssetVersionMarker.Read(MarkerPath)!;
            Assert.Equal("2.0.0", read.Version);
            Assert.Equal([new KeyValuePair<AssetGroup, int>(AssetGroup.Video, 5)], read.Revisions);
            Assert.Equal([MarkerPath], Directory.GetFiles(_root.Path));
        }

        [Fact]
        public void AMarkerThatCannotBeReplacedFailsTheWriteAndLeavesNoTemporaryFile()
        {
            Directory.CreateDirectory(MarkerPath);

            var failure = Assert.Throws<IOException>(() => AssetVersionMarker.Write(MarkerPath, Assets("1.0.0")));

            Assert.Contains(MarkerPath, failure.Message, StringComparison.Ordinal);
            Assert.Empty(Directory.GetFiles(_root.Path));
        }

        [Theory]
        [InlineData("4.5.6")]
        [InlineData("0.1.0-alpha")]
        [InlineData("nightly")]
        public void AWrittenVersionReadsBackUnchanged(string written)
        {
            AssetVersionMarker.Write(MarkerPath, Assets(written));

            Assert.Equal(written, AssetVersionMarker.Read(MarkerPath)?.Version);
        }

        [Theory]
        [InlineData("")]
        [InlineData("1.0+build.5")]
        [InlineData("a;b")]
        [InlineData("0.1.0 alpha")]
        [InlineData("-x")]
        [InlineData("1.0\n")]
        public void AnInvalidVersionIsNotWrittenAndLeavesNoFile(string version)
        {
            Assert.Throws<ArgumentException>(() => AssetVersionMarker.Write(MarkerPath, Assets(version)));

            Assert.Empty(Directory.GetFiles(_root.Path));
        }

        [Theory]
        [InlineData(0)]
        [InlineData(-1)]
        public void ANonPositiveRevisionIsNotWrittenAndLeavesNoFile(int revision)
        {
            Assert.Throws<ArgumentException>(() => AssetVersionMarker.Write(MarkerPath, Assets("1.0.0", (AssetGroup.Audio, revision))));

            Assert.Empty(Directory.GetFiles(_root.Path));
        }

        [Theory]
        [InlineData("something else")]
        [InlineData("")]
        [InlineData("\n")]
        [InlineData("Asset Version")]
        [InlineData("Asset Version ")]
        [InlineData("Asset Version 0.1.0 alpha")]
        [InlineData("Asset Version 1.0+build.5")]
        [InlineData("Asset Version -x")]
        [InlineData("asset version 1.2.3")]
        [InlineData("Asset Version 1.2.3\nsecond line")]
        [InlineData("Asset Version 1.2.3\nshaders")]
        [InlineData("Asset Version 1.2.3\nshaders=")]
        [InlineData("Asset Version 1.2.3\n=1")]
        [InlineData("Asset Version 1.2.3\nshaders=x")]
        [InlineData("Asset Version 1.2.3\nshaders=1.5")]
        [InlineData("Asset Version 1.2.3\nshaders=0")]
        [InlineData("Asset Version 1.2.3\nshaders=-1")]
        [InlineData("Asset Version 1.2.3\nshaders=99999999999")]
        [InlineData("Asset Version 1.2.3\nshaders = 1")]
        [InlineData("Asset Version 1.2.3\nshaders=1\n\naudio=1")]
        [InlineData("Asset Version 1.2.3\nshaders=1\n\n")]
        [InlineData("Asset Version 1.2.3\nshaders=1\nshaders=1")]
        [InlineData("Asset Version 1.2.3\nshaders=1\nshaders=2")]
        [InlineData("Asset Version 1.2.3\nfuture=1\nfuture=1")]
        public void AnythingBeyondAVersionLineAndWellFormedGroupLinesIsNoRecordNotAnError(string content)
        {
            File.WriteAllText(MarkerPath, content);

            Assert.Null(AssetVersionMarker.Read(MarkerPath));
        }

        [Theory]
        [InlineData("Asset Version 1.2.3")]
        [InlineData("Asset Version 1.2.3\n")]
        [InlineData("Asset Version 1.2.3\r\n")]
        public void ABareVersionLineFromBeforeTheGroupsExistedIsARecordOfNoGroup(string content)
        {
            var parsed = AssetVersionMarker.Parse(content);

            Assert.Equal("1.2.3", parsed?.Version);
            Assert.Empty(parsed!.Revisions);
        }

        [Theory]
        [InlineData("Asset Version 1.2.3\nshaders=3\naudio=2")]
        [InlineData("Asset Version 1.2.3\nshaders=3\naudio=2\n")]
        [InlineData("Asset Version 1.2.3\r\nshaders=3\r\naudio=2\r\n")]
        public void ALineEndingOrItsAbsenceDoesNotMatter(string content)
        {
            var parsed = AssetVersionMarker.Parse(content);

            Assert.Equal("1.2.3", parsed?.Version);
            Assert.Equal(
                [new KeyValuePair<AssetGroup, int>(AssetGroup.Shaders, 3), new KeyValuePair<AssetGroup, int>(AssetGroup.Audio, 2)],
                parsed!.Revisions.OrderBy(r => r.Key));
        }

        [Fact]
        public void UnknownGroupIdsAreIgnoredAndNotWrittenBack()
        {
            var parsed = AssetVersionMarker.Parse("Asset Version 1.2.3\nshaders=3\nfuture-group=7\n");

            Assert.Equal([new KeyValuePair<AssetGroup, int>(AssetGroup.Shaders, 3)], parsed!.Revisions);
            Assert.Equal("Asset Version 1.2.3\nshaders=3\n", AssetVersionMarker.Format(parsed));
        }

        [Fact]
        public void UpdatingGivesTheChangeTheCurrentRecordAndWritesWhatItReturns()
        {
            AssetVersionMarker.Write(MarkerPath, Assets("1.0.0", (AssetGroup.Audio, 1)));

            AssetVersionMarker.Update(MarkerPath, current => current!.Recording(AssetGroup.Video, 2));

            Assert.Equal("Asset Version 1.0.0\naudio=1\nvideo=2\n", File.ReadAllText(MarkerPath));
        }

        [Fact]
        public void UpdatingToNothingEmptiesTheMarker()
        {
            AssetVersionMarker.Write(MarkerPath, Assets("1.0.0", (AssetGroup.Audio, 1)));

            AssetVersionMarker.Update(MarkerPath, _ => null);

            Assert.Empty(File.ReadAllBytes(MarkerPath));
        }

        [Fact]
        public void UpdatesFromSeveralThreadsAllSurvive()
        {
            AssetVersionMarker.Write(MarkerPath, Assets("1.0.0"));

            Parallel.ForEach(AssetGroups.All, group => AssetVersionMarker.Update(MarkerPath, current => current!.Recording(group, 1)));

            Assert.Equal(AssetGroups.All.Order(), AssetVersionMarker.Read(MarkerPath)!.Revisions.Keys.Order());
        }
    }

    public sealed class InstallLocationsMarkerTests : IDisposable
    {
        private readonly TempDirectory _root = new();

        public void Dispose() => _root.Dispose();

        private InstallLocations Locations => new(Path.Combine(_root.Path, "game"), Path.Combine(_root.Path, "install"));

        private InstalledAssets? Marker() => AssetVersionMarker.Read(Locations.OwnershipMarker);

        private static ReconversionPlan PlanReconverting(params AssetGroup[] groups)
        {
            var installed = new InstalledAssets("1.0.0", AssetGroups.CurrentRevisions);
            var current = AssetGroups.CurrentRevisions.ToDictionary(r => r.Key, r => groups.Contains(r.Key) ? r.Value + 1 : r.Value);
            return ReconversionPlan.Decide(installed, current);
        }

        [Fact]
        public void StartingAnInstallClaimsTheFolderWithAnEmptyMarkerThatRecordsNoGroup()
        {
            Locations.CreateOutputDirectories();

            Assert.True(Locations.HoldsInstall());
            Assert.Empty(File.ReadAllBytes(Locations.OwnershipMarker));
            Assert.Null(Marker());
        }

        [Fact]
        public void StartingAgainOverACompleteInstallKeepsItsRecord()
        {
            Locations.CreateOutputDirectories();
            AssetVersionMarker.Write(Locations.OwnershipMarker, new InstalledAssets("1.0.0", AssetGroups.CurrentRevisions));

            Locations.CreateOutputDirectories();

            Assert.Equal("1.0.0", Marker()?.Version);
            Assert.Equal(AssetGroups.All.Order(), Marker()!.Revisions.Keys.Order());
        }

        [Fact]
        public void WithdrawingListsOnlyTheKeptGroupsAtTheirInstalledRevisionsUnderTheNewVersion()
        {
            Locations.CreateOutputDirectories();
            AssetVersionMarker.Write(Locations.OwnershipMarker, new InstalledAssets("1.0.0", new Dictionary<AssetGroup, int>
            {
                [AssetGroup.Shaders] = 1,
                [AssetGroup.Audio] = 1,
                [AssetGroup.Meshes] = 1,
                [AssetGroup.Animations] = 1,
                [AssetGroup.TexturesDatabase] = 1,
            }));

            Locations.WithdrawReconverted(PlanReconverting(AssetGroup.Meshes), "2.0.0");

            Assert.Equal("2.0.0", Marker()?.Version);
            Assert.Equal([AssetGroup.Shaders, AssetGroup.Audio], Marker()!.Revisions.Keys.Order());
        }

        [Fact]
        public void WithdrawingEverythingEmptiesTheMarker()
        {
            Locations.CreateOutputDirectories();
            AssetVersionMarker.Write(Locations.OwnershipMarker, new InstalledAssets("1.0.0", AssetGroups.CurrentRevisions));

            Locations.WithdrawReconverted(ReconversionPlan.ReconvertAll, "2.0.0");

            Assert.Empty(File.ReadAllBytes(Locations.OwnershipMarker));
        }

        [Fact]
        public void WithdrawingOverAnUnreadableMarkerEmptiesIt()
        {
            Locations.CreateOutputDirectories();
            File.WriteAllText(Locations.OwnershipMarker, "something else");

            Locations.WithdrawReconverted(PlanReconverting(), "2.0.0");

            Assert.Empty(File.ReadAllBytes(Locations.OwnershipMarker));
        }

        [Fact]
        public void FinishedGroupsAccumulateInTheMarkerAtTheirCurrentRevision()
        {
            Locations.CreateOutputDirectories();

            Locations.MarkGroupComplete(AssetGroup.Audio, "2.0.0");
            Locations.MarkGroupComplete(AssetGroup.Meshes, "2.0.0");

            Assert.Equal("2.0.0", Marker()?.Version);
            Assert.Equal([AssetGroup.Audio, AssetGroup.Meshes], Marker()!.Revisions.Keys.Order());
            Assert.Equal(AssetGroups.Revision(AssetGroup.Meshes), Marker()!.Revisions[AssetGroup.Meshes]);
        }

        [Fact]
        public void AFinishedGroupIsAddedToTheKeptOnesAfterAWithdrawal()
        {
            Locations.CreateOutputDirectories();
            AssetVersionMarker.Write(Locations.OwnershipMarker, new InstalledAssets("1.0.0", AssetGroups.CurrentRevisions));
            Locations.WithdrawReconverted(PlanReconverting(AssetGroup.Collision), "2.0.0");

            Locations.MarkGroupComplete(AssetGroup.Collision, "2.0.0");

            Assert.Equal(AssetGroups.All.Order(), Marker()!.Revisions.Keys.Order());
        }

        [Fact]
        public void AMarkerOfAnyContentStillClaimsTheFolderAsAnInstall()
        {
            Directory.CreateDirectory(Locations.InstallRoot);
            File.WriteAllText(Locations.OwnershipMarker, "something else");

            Assert.True(Locations.HoldsInstall());
            Assert.Null(Marker());
        }
    }
}
