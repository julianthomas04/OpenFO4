using AssetConverter.Audio;
using Xunit;

namespace InstallerTests.Audio
{
    public sealed class AudioPlanTests
    {
        private static IReadOnlyList<IReadOnlyList<AudioEntry>> Plan(params string[][] archives) =>
            AudioPlan.Build([.. archives.Select(paths => (IEnumerable<string>)paths)]);

        private static string[] Paths(IReadOnlyList<AudioEntry> entries) => [.. entries.Select(entry => entry.Path).Order(StringComparer.Ordinal)];

        [Fact]
        public void EachArchiveConvertsItsOwnSoundAndMusicEntries()
        {
            var plan = Plan([@"Sound\fx\a.xwm", @"Strings\x.strings", @"Music\m.wav"], [@"Sound\fx\b.wav"]);

            Assert.Equal([@"Music\m.wav", @"Sound\fx\a.xwm"], Paths(plan[0]));
            Assert.Equal([@"Sound\fx\b.wav"], Paths(plan[1]));
            Assert.Equal(new AudioEntry(@"Music\m.wav", "music/m.flac", AudioContainer.Wav), plan[0].Single(entry => entry.Path == @"Music\m.wav"));
        }

        [Fact]
        public void VoiceFilesAreLeftOut()
        {
            var plan = Plan([@"Sound\voice\fallout4.esm\npc\line.fuz", @"Sound\fx\a.xwm"]);

            Assert.Equal([@"Sound\fx\a.xwm"], Paths(plan[0]));
        }

        [Fact]
        public void APathInSeveralArchivesIsConvertedOnlyByTheLastOne()
        {
            var plan = Plan([@"Music\Special\MUS_MainTheme.xwm", @"Sound\a.xwm"], [@"music\special\mus_maintheme.XWM"]);

            Assert.Equal([@"Sound\a.xwm"], Paths(plan[0]));
            Assert.Equal([@"music\special\mus_maintheme.XWM"], Paths(plan[1]));
        }

        [Fact]
        public void APathListedTwiceInOneArchiveIsConvertedOnce()
        {
            var plan = Plan([@"Sound\a.xwm", "Sound/A.xwm"]);

            Assert.Single(plan[0]);
        }

        [Fact]
        public void TwoDifferentSourcesForOneOutputFailAndTheMessageNamesBoth()
        {
            var error = Assert.Throws<AudioConversionException>(() => Plan([@"Sound\fx\boom.wav", @"Sound\fx\boom.xwm"]));

            Assert.Contains(@"'Sound\fx\boom.wav'", error.Message, StringComparison.Ordinal);
            Assert.Contains(@"'Sound\fx\boom.xwm'", error.Message, StringComparison.Ordinal);
            Assert.Contains("audio/sound/fx/boom.flac", error.Message, StringComparison.Ordinal);
        }

        [Fact]
        public void TwoSourcesForOneOutputFailAcrossArchivesToo()
        {
            Assert.Throws<AudioConversionException>(() => Plan([@"Sound\fx\boom.wav"], [@"SOUND\FX\BOOM.XWM"]));
        }

        [Fact]
        public void AnEntryThatIsNeitherXwmNorWavFailsTheWholePlan()
        {
            var error = Assert.Throws<AudioConversionException>(() => Plan([@"Sound\fx\a.xwm", @"Sound\fx\notes.txt"]));

            Assert.Contains("notes.txt", error.Message, StringComparison.Ordinal);
        }
    }
}
