using AssetConverter;
using Mutagen.Bethesda.Fallout4;
using Xunit;

namespace InstallerTests
{
    /// <summary>
    /// Checks the ambient-music rows against the Fallout4.esm of the install named by <c>OPENFO4_FO4_PATH</c>, reading the plugin directly
    /// (no conversion run). Explicit: run with <c>-- xUnit.Explicit=only</c>; it never runs in the regular suite.
    /// </summary>
    public sealed class EsmMusicRealInstallTests : IDisposable
    {
        const int MusicTrackCount = 552;
        const int MusicTypeCount = 111;
        const int MusicCellCount = 281;
        const int LocationMusicCount = 67;
        const int GlobalCount = 1346;
        const float ExplorePriority = 90;
        const float ExploreFadeSeconds = 12;

        readonly IFallout4ModDisposableGetter _file;

        public EsmMusicRealInstallTests()
        {
            string? game = Environment.GetEnvironmentVariable("OPENFO4_FO4_PATH");
            Assert.SkipWhen(string.IsNullOrEmpty(game), "OPENFO4_FO4_PATH names no Fallout 4 install.");
            _file = Fallout4Mod.CreateFromBinaryOverlay(Path.Combine(game!, "Data", "Fallout4.esm"), Fallout4Release.Fallout4);
        }

        public void Dispose() => _file.Dispose();

        [Fact(Explicit = true)]
        public void TheBaseGameHasTheDocumentedRecordCounts()
        {
            Assert.Equal(MusicTrackCount, _file.MusicTracks.Count);
            Assert.Equal(MusicTypeCount, _file.MusicTypes.Count);
            Assert.Equal(GlobalCount, _file.Globals.Count);
            Assert.Equal(LocationMusicCount, ESMLoader.LocationMusicRows(_file).Count);
            Assert.Equal(MusicCellCount, ESMLoader.CellPlaceRows(_file).Count(row => row[1] is not null));
        }

        [Fact(Explicit = true)]
        public void TheExploreMusicTypeHasTheDocumentedPriorityAndFade()
        {
            var explore = _file.MusicTypes.Single(type => type.EditorID == "MUSzExplore");

            Assert.Equal(ExplorePriority, explore.Data!.Priority);
            Assert.Equal(ExploreFadeSeconds, explore.FadeDuration);
        }

        [Fact(Explicit = true)]
        public void EveryMusicTypeHasAPriorityAndEverySingleTrackNamesAFile()
        {
            Assert.All(_file.MusicTypes, type => Assert.NotNull(type.Data));
            Assert.All(_file.MusicTracks.Where(track => track.Type == MusicTrack.TypeEnum.SingleTrack), track => Assert.False(string.IsNullOrEmpty(track.TrackFilename), track.EditorID));
        }
    }
}
