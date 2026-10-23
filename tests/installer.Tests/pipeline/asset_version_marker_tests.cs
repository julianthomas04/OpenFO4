using AssetConverter.Pipeline;
using Xunit;

namespace InstallerTests.Pipeline
{
    public sealed class AssetVersionMarkerTests : IDisposable
    {
        private readonly TempDirectory _root = new();

        public void Dispose() => _root.Dispose();

        private string MarkerPath => Path.Combine(_root.Path, "marker");

        [Fact]
        public void TheMarkerIsExactlyTheVersionLineWithoutANewlineOrAByteOrderMark()
        {
            AssetVersionMarker.Write(MarkerPath, "1.22.333");

            Assert.Equal("Asset Version 1.22.333"u8.ToArray(), File.ReadAllBytes(MarkerPath));
        }

        [Fact]
        public void ThePendingMarkerIsEmptyAndReadsAsAnUnknownVersion()
        {
            AssetVersionMarker.WritePending(MarkerPath);

            Assert.Empty(File.ReadAllBytes(MarkerPath));
            Assert.False(AssetVersionMarker.TryRead(MarkerPath, out _));
        }

        [Fact]
        public void ADevelopmentBuildsVersionZeroIsARealVersionNotPending()
        {
            AssetVersionMarker.Write(MarkerPath, "0.0.0-dev");

            Assert.True(AssetVersionMarker.TryRead(MarkerPath, out var version));
            Assert.Equal("0.0.0-dev", version);
        }

        [Fact]
        public void WritingReplacesAnEarlierMarkerAndLeavesNoTemporaryFile()
        {
            AssetVersionMarker.Write(MarkerPath, "1.0.0");

            AssetVersionMarker.Write(MarkerPath, "2.0.0");

            Assert.True(AssetVersionMarker.TryRead(MarkerPath, out var version));
            Assert.Equal("2.0.0", version);
            Assert.Equal([MarkerPath], Directory.GetFiles(_root.Path));
        }

        [Fact]
        public void AMarkerThatCannotBeReplacedFailsTheWriteAndLeavesNoTemporaryFile()
        {
            Directory.CreateDirectory(MarkerPath);

            var failure = Assert.Throws<IOException>(() => AssetVersionMarker.Write(MarkerPath, "1.0.0"));

            Assert.Contains(MarkerPath, failure.Message, StringComparison.Ordinal);
            Assert.Empty(Directory.GetFiles(_root.Path));
        }

        [Theory]
        [InlineData("4.5.6")]
        [InlineData("0.1.0-alpha")]
        [InlineData("nightly")]
        public void AWrittenVersionReadsBackUnchanged(string written)
        {
            AssetVersionMarker.Write(MarkerPath, written);

            Assert.True(AssetVersionMarker.TryRead(MarkerPath, out var version));
            Assert.Equal(written, version);
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
            Assert.Throws<ArgumentException>(() => AssetVersionMarker.Write(MarkerPath, version));

            Assert.Empty(Directory.GetFiles(_root.Path));
        }

        [Fact]
        public void AMissingFileIsAnUnknownVersion()
        {
            Assert.False(AssetVersionMarker.TryRead(MarkerPath, out var version));
            Assert.Null(version);
        }

        [Theory]
        [InlineData("something else")]
        [InlineData("")]
        [InlineData("Asset Version")]
        [InlineData("Asset Version ")]
        [InlineData("Asset Version 0.1.0 alpha")]
        [InlineData("Asset Version 1.0+build.5")]
        [InlineData("Asset Version -x")]
        [InlineData("asset version 1.2.3")]
        [InlineData("Asset Version 1.2.3\nsecond line")]
        public void AnythingButTheVersionLineIsAnUnknownVersionNotAnError(string content)
        {
            File.WriteAllText(MarkerPath, content);

            Assert.False(AssetVersionMarker.TryRead(MarkerPath, out var version));
            Assert.Null(version);
        }

        [Theory]
        [InlineData("Asset Version 1.2.3")]
        [InlineData("Asset Version 1.2.3\n")]
        [InlineData("Asset Version 1.2.3\r\n")]
        public void ALineEndingOrItsAbsenceDoesNotMatter(string content)
        {
            Assert.True(AssetVersionMarker.TryParse(content, out var version));
            Assert.Equal("1.2.3", version);
        }
    }

    public sealed class InstallLocationsMarkerTests : IDisposable
    {
        private readonly TempDirectory _root = new();

        public void Dispose() => _root.Dispose();

        private InstallLocations Locations => new(Path.Combine(_root.Path, "game"), Path.Combine(_root.Path, "install"));

        private string? MarkerVersion() => AssetVersionMarker.TryRead(Locations.OwnershipMarker, out var version) ? version : null;

        [Fact]
        public void StartingAnInstallClaimsTheFolderWithAnEmptyMarkerThatVouchesForNoAssetVersion()
        {
            Locations.CreateOutputDirectories();

            Assert.True(Locations.HoldsInstall());
            Assert.Empty(File.ReadAllBytes(Locations.OwnershipMarker));
            Assert.Null(MarkerVersion());
        }

        [Fact]
        public void StartingAgainOverACompleteInstallWithdrawsItsVersionBecauseTheAssetsAreAboutToChange()
        {
            Locations.CreateOutputDirectories();
            Locations.MarkAssetsComplete("1.0.0");

            Locations.CreateOutputDirectories();

            Assert.Null(MarkerVersion());
        }

        [Fact]
        public void CompletingTheAssetsRecordsTheVersion()
        {
            Locations.CreateOutputDirectories();

            Locations.MarkAssetsComplete("2.3.4");

            Assert.Equal("2.3.4", MarkerVersion());
        }

        [Fact]
        public void AMarkerOfAnyContentStillClaimsTheFolderAsAnInstall()
        {
            Directory.CreateDirectory(Locations.InstallRoot);
            File.WriteAllText(Locations.OwnershipMarker, "something else");

            Assert.True(Locations.HoldsInstall());
            Assert.False(AssetVersionMarker.TryRead(Locations.OwnershipMarker, out _));
        }
    }
}
