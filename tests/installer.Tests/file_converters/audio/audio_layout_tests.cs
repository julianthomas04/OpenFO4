using AssetConverter;
using AssetConverter.Audio;
using Xunit;

namespace InstallerTests.Audio
{
    public sealed class AudioLayoutTests
    {
        [Fact]
        public void TheKeyOfTheContractExampleIsLowercaseWithForwardSlashesAndAFlacExtension()
        {
            Assert.Equal("music/special/mus_maintheme.flac", AudioLayout.ToAudioKey(@"Music\Special\MUS_MainTheme.xwm"));
        }

        [Theory]
        [InlineData(@"SOUND\FX\UI\Click.WAV", "sound/fx/ui/click.flac")]
        [InlineData("Sound/FX/Click.XWM", "sound/fx/click.flac")]
        [InlineData(@"Sound\a.b\c.d.wav", "sound/a.b/c.d.flac")]
        public void TheKeyIgnoresCaseSlashDirectionAndTheCasingOfTheExtension(string archivePath, string expected)
        {
            Assert.Equal(expected, AudioLayout.ToAudioKey(archivePath));
        }

        [Theory]
        [InlineData(@"data\Music\Special\MUS_MainTheme.wav")]
        [InlineData(@"\Data\Music\Special\MUS_MainTheme.wav")]
        [InlineData(@"Music\Special\MUS_MainTheme.wav")]
        public void ARecordPathFromTheGameFolderHasTheKeyOfTheArchiveEntry(string recordPath)
        {
            Assert.Equal(AudioLayout.ToAudioKey(@"Music\Special\MUS_MainTheme.xwm"), AudioLayout.ToAudioKey(recordPath));
        }

        [Theory]
        [InlineData(@"Sound\fx\readme.txt")]
        [InlineData(@"Sound\fx\noextension")]
        [InlineData(@"Music\theme.mp3")]
        public void AnExtensionOtherThanXwmOrWavIsAnError(string archivePath)
        {
            var error = Assert.Throws<AudioConversionException>(() => AudioLayout.ToAudioKey(archivePath));

            Assert.Contains(archivePath, error.Message, StringComparison.Ordinal);
        }

        [Theory]
        [InlineData(@"Voice\fo4.esm\line.wav")]
        [InlineData(@"data\Textures\hit.wav")]
        [InlineData(@"loose.wav")]
        [InlineData(@"MusicBox\theme.wav")]
        public void APathOutsideSoundAndMusicIsAnError(string recordPath)
        {
            var error = Assert.Throws<AudioConversionException>(() => AudioLayout.ToAudioKey(recordPath));

            Assert.Contains(recordPath, error.Message, StringComparison.Ordinal);
        }

        [Theory]
        [InlineData(@"Sound\..\..\escape.wav")]
        [InlineData(@"Sound\\double.wav")]
        public void APathThatLeavesTheAudioFolderIsAnError(string archivePath)
        {
            Assert.Throws<AudioConversionException>(() => AudioLayout.ToAudioKey(archivePath));
        }

        [Theory]
        [InlineData(@"Sound\fx\a.xwm", true)]
        [InlineData(@"Music\b.WAV", false)]
        public void TheContainerFollowsTheExtension(string archivePath, bool isXwma)
        {
            Assert.Equal(isXwma ? AudioContainer.Xwma : AudioContainer.Wav, AudioLayout.ContainerOf(archivePath));
        }

        [Theory]
        [InlineData(@"Sound\fx\a.xwm", true)]
        [InlineData("music/a.wav", true)]
        [InlineData(@"SOUND\a.wav", true)]
        [InlineData(@"Sound\voice\fallout4.esm\npc\line.fuz", false)]
        [InlineData(@"Sound\fx\LINE.FUZ", false)]
        [InlineData(@"Strings\fallout4_en.strings", false)]
        [InlineData(@"Soundtrack\a.wav", false)]
        public void OnlySoundAndMusicEntriesWithoutVoiceFilesAreInScope(string archivePath, bool expected)
        {
            Assert.Equal(expected, AudioLayout.IsInScope(archivePath));
        }

        [Fact]
        public void TheOutputIsPlacedBelowTheAudioRootAndTheMarkerBesideIt()
        {
            string root = Path.Combine("install", "assets", InstallLayout.AudioFolder);

            Assert.Equal(Path.Combine(root, "music", "special", "mus_maintheme.flac"), AudioLayout.OutputPath(root, "music/special/mus_maintheme.flac"));
            Assert.Equal(Path.Combine(root, ".Fallout4 - Sounds.ba2.stamp"), AudioLayout.MarkerPath(root, "Fallout4 - Sounds.ba2"));
        }

        [Fact]
        public void TheSourcesAreTheSoundsAndStartupArchivesInThatOrder()
        {
            Assert.Equal(["Fallout4 - Sounds.ba2", "Fallout4 - Startup.ba2"], AudioLayout.Archives);
            Assert.Equal("audio", InstallLayout.AudioFolder);
        }
    }
}
