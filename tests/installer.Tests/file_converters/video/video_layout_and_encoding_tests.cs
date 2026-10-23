using AssetConverter;
using AssetConverter.Video;
using Xunit;

namespace InstallerTests.Video
{
    public sealed class VideoLayoutAndEncodingTests
    {
        private static VideoInfo Info(int audioTracks, int width = 1920, int height = 1080, int numerator = 24000, int denominator = 1001) =>
            new(width, height, 4000, numerator, denominator, [.. Enumerable.Range(0, audioTracks).Select(i => new AudioTrackInfo(i, 48000, 2))]);

        [Fact]
        public void TheOutputIsTheLowercasedBaseNameWithTheMp4ExtensionBelowTheVideoFolder()
        {
            Assert.Equal(Path.Combine("out", "mainmenuloop.mp4"), VideoLayout.OutputPath("out", Path.Combine("game", "Data", "Video", "MainMenuLoop.BK2")));
            Assert.Equal("video", InstallLayout.VideoFolder);
        }

        [Fact]
        public void TracksBeyondTheLanguageTableAreUndetermined()
        {
            Assert.Equal(["eng", "fra", "ita", "deu", "spa", "und", "und"], Enumerable.Range(0, 7).Select(VideoLayout.LanguageOfTrack));
        }

        [Fact]
        public void TheArgumentsAreExactlyThePinnedConversionOfASixTrackVideo()
        {
            string[] expected =
            [
                "-nostdin", "-v", "error", "-nostats", "-progress", "pipe:1", "-xerror",
                "-f", "rawvideo", "-pixel_format", "yuv420p", "-video_size", "1920x1080", "-framerate", "24000/1001", "-color_range", "pc", "-colorspace", "smpte170m", "-i", "pipe:0",
                "-err_detect", "explode", "-i", "in.bk2",
                "-map", "0:v:0", "-map", "1:a:0", "-map", "1:a:1", "-map", "1:a:2", "-map", "1:a:3", "-map", "1:a:4", "-map", "1:a:5",
                "-vf", "scale=in_range=full:out_range=limited:in_color_matrix=bt601:out_color_matrix=bt601",
                "-c:v", "libx264", "-preset", "medium", "-crf", "18", "-pix_fmt", "yuv420p",
                "-color_range", "tv", "-colorspace", "smpte170m", "-color_primaries", "smpte170m", "-color_trc", "smpte170m",
                "-c:a", "aac", "-b:a", "192k",
                "-metadata:s:a:0", "language=eng", "-metadata:s:a:1", "language=fra", "-metadata:s:a:2", "language=ita",
                "-metadata:s:a:3", "language=deu", "-metadata:s:a:4", "language=spa", "-metadata:s:a:5", "language=und",
                "-map_metadata", "-1", "-bitexact", "-movflags", "+faststart+write_colr", "-f", "mp4", "out.mp4.tmp",
            ];

            Assert.Equal(expected, VideoEncoding.Arguments(Info(6), "in.bk2", "out.mp4.tmp"));
        }

        [Fact]
        public void AVideoWithoutAudioMapsAndTagsNoAudioTrack()
        {
            var arguments = VideoEncoding.Arguments(Info(0, numerator: 30, denominator: 1), "loop.bk2", "loop.mp4.tmp");

            Assert.DoesNotContain(arguments, argument => argument.StartsWith("1:a", StringComparison.Ordinal) || argument.StartsWith("-metadata:s:a", StringComparison.Ordinal));
            Assert.Equal("30/1", arguments[arguments.ToList().IndexOf("-framerate") + 1]);
        }

        /// <summary>Pinned: any change to an encode argument changes the fingerprint, which converts every video again; update the value with the change, on purpose.</summary>
        [Fact]
        public void TheSettingsFingerprintIsThePinnedHashOfTheEncodeArguments()
        {
            Assert.Equal("50666f6984570b58e330405b479de0832df6e4c32e5e5d27276600f375627523", VideoEncoding.SettingsFingerprint);
        }

        [Fact]
        public void TheSettingsFingerprintCoversTheLanguageTable()
        {
            Assert.Equal(VideoEncoding.SettingsFingerprint, VideoEncoding.Fingerprint(VideoLayout.AudioLanguages));
            Assert.NotEqual(VideoEncoding.SettingsFingerprint, VideoEncoding.Fingerprint(["fra", "eng", "ita", "deu", "spa"]));
            Assert.NotEqual(VideoEncoding.SettingsFingerprint, VideoEncoding.Fingerprint([.. VideoLayout.AudioLanguages, "jpn"]));
            Assert.NotEqual(VideoEncoding.SettingsFingerprint, VideoEncoding.Fingerprint(VideoLayout.AudioLanguages.Take(4).ToArray()));
        }

        [Theory]
        [InlineData("out_time_us=125000\nprogress=continue\nout_time_us=4167000\nprogress=end\n", 4.167)]
        [InlineData("out_time_us=N/A\nprogress=end\n", null)]
        [InlineData("", null)]
        public void TheOutputSecondsAreTheLastOutTimeOfTheProgress(string progress, double? expected)
        {
            Assert.Equal(expected, VideoEncoding.OutputSeconds(progress));
        }

        [Fact]
        public void AnAudioTrackIsMeasuredByCopyingItToNowhere()
        {
            Assert.Equal(
                ["-nostdin", "-v", "error", "-nostats", "-progress", "pipe:1", "-xerror", "-f", "mp4", "-i", "v.mp4", "-map", "0:a:2", "-c", "copy", "-f", "null", "-"],
                VideoEncoding.AudioDurationArguments("v.mp4", 2));
        }

        [Theory]
        [InlineData(10.0, 10.0, true)]
        [InlineData(10.24, 10.0, true)]
        [InlineData(9.76, 10.0, true)]
        [InlineData(10.3, 10.0, false)]
        [InlineData(9.7, 10.0, false)]
        public void AnAudioTrackMustLastAsLongAsItsVideoWithinTheTolerance(double audio, double video, bool accepted)
        {
            if (accepted) VideoTranscoding.CheckAudioDuration("a.bk2", 0, audio, video);
            else Assert.Contains("Audio track 0 of 'a.bk2'", Assert.Throws<VideoConversionException>(() => VideoTranscoding.CheckAudioDuration("a.bk2", 0, audio, video)).Message, StringComparison.Ordinal);
        }

        [Theory]
        [InlineData("frame=0\nprogress=continue\nframe=12\nprogress=continue\nframe=4000\nprogress=end\n", 4000)]
        [InlineData("frame=7\r\nprogress=end\r\n", 7)]
        [InlineData("progress=end\n", null)]
        [InlineData("", null)]
        public void TheEncodedFramesAreTheLastFrameLineOfTheProgress(string progress, int? expected)
        {
            Assert.Equal(expected, VideoEncoding.EncodedFrames(progress));
        }

        [Fact]
        public void InfoIsReadInAnyLetterCase()
        {
            const string Json = """{"Width":1920,"Height":1080,"FrameCount":4026,"FrameRateNumerator":24,"FrameRateDenominator":1,"AudioTracks":[{"Id":0,"SampleRate":48000,"Channels":2},{"id":1,"sampleRate":48000,"channels":2}]}""";

            var info = VideoInfo.Parse(Json);

            Assert.Equal(4026, info.FrameCount);
            Assert.Equal(2, info.AudioTracks.Count);
            Assert.Equal(1920L * 1080 * 3 / 2, info.FrameBytes);
        }

        [Theory]
        [InlineData("not json")]
        [InlineData("""{"width":1920}""")]
        [InlineData("""{"width":0,"height":1080,"frameCount":1,"frameRateNumerator":24,"frameRateDenominator":1,"audioTracks":[]}""")]
        [InlineData("""{"width":1921,"height":1080,"frameCount":1,"frameRateNumerator":24,"frameRateDenominator":1,"audioTracks":[]}""")]
        [InlineData("""{"width":1920,"height":1080,"frameCount":"many","frameRateNumerator":24,"frameRateDenominator":1,"audioTracks":[]}""")]
        [InlineData("[]")]
        public void MalformedInfoFailsExplicitly(string json)
        {
            Assert.Throws<VideoConversionException>(() => VideoInfo.Parse(json));
        }

        [Fact]
        public void TheVerificationFailsLoudlyOnAShortDecoderOrAnEncoderThatWroteOtherFrames()
        {
            var info = new VideoInfo(2, 2, 3, 24, 1, []);

            VideoTranscoding.VerifyFrames("a.bk2", info, 18, 3);
            Assert.Contains("fed 12", Assert.Throws<VideoConversionException>(() => VideoTranscoding.VerifyFrames("a.bk2", info, 12, 3)).Message, StringComparison.Ordinal);
            Assert.Contains("wrote 2 frames", Assert.Throws<VideoConversionException>(() => VideoTranscoding.VerifyFrames("a.bk2", info, 18, 2)).Message, StringComparison.Ordinal);
            Assert.Contains("unknown number", Assert.Throws<VideoConversionException>(() => VideoTranscoding.VerifyFrames("a.bk2", info, 18, null)).Message, StringComparison.Ordinal);
        }
    }
}
