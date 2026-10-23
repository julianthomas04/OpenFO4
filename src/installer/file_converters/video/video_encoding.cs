using System.Globalization;
using System.Security.Cryptography;
using System.Text;

namespace AssetConverter.Video
{
    /// <summary>
    /// How a decoded video becomes an MP4 by ffmpeg: the decoder's full-range BT.601 4:2:0 frames arrive raw on stdin and are scaled to
    /// limited range (same matrix) for H.264 (CRF 18, medium) tagged BT.601; the audio tracks are read from the Bink file itself and each
    /// is encoded to AAC and tagged with its language. The colour description is also written to the container (<c>colr</c>), where
    /// Media Foundation reads it. The output carries no other metadata and its index leads the file.
    /// Progress is reported on stdout as key=value lines (<c>-progress</c>); damaged input is a nonzero exit.
    /// </summary>
    internal static class VideoEncoding
    {
        /// <summary>Damaged input is a nonzero exit rather than a partial file: <c>-xerror</c> for the run, <c>-err_detect explode</c> for the Bink file's own demuxing. The one place to relax if a real file trips it.</summary>
        private static readonly string[] StrictRun = ["-xerror"];
        private static readonly string[] StrictInput = ["-err_detect", "explode"];

        private static readonly string[] Quiet = ["-nostdin", "-v", "error", "-nostats", "-progress", "pipe:1", .. StrictRun];
        private static readonly string[] Range = ["-vf", "scale=in_range=full:out_range=limited:in_color_matrix=bt601:out_color_matrix=bt601"];
        private static readonly string[] Video =
        [
            "-c:v", "libx264", "-preset", "medium", "-crf", "18", "-pix_fmt", "yuv420p",
            "-color_range", "tv", "-colorspace", "smpte170m", "-color_primaries", "smpte170m", "-color_trc", "smpte170m",
        ];
        private static readonly string[] Audio = ["-c:a", "aac", "-b:a", "192k"];
        private static readonly string[] Container = ["-map_metadata", "-1", "-bitexact", "-movflags", "+faststart+write_colr", "-f", "mp4"];

        /// <summary>Stands for every argument of the conversion except the paths and the properties of the video; part of the conversion stamp, so a change of arguments converts again.</summary>
        public static string SettingsFingerprint { get; } = Fingerprint(VideoLayout.AudioLanguages);

        /// <summary>The hash of the arguments for a video with one track beyond <paramref name="languages"/>, with placeholders for size, rate and paths.</summary>
        internal static string Fingerprint(IReadOnlyList<string> languages)
        {
            string settings = string.Join(' ', Build("{size}", "{rate}", languages.Count + 1, "", "", languages));
            return Convert.ToHexString(SHA256.HashData(Encoding.UTF8.GetBytes(settings))).ToLowerInvariant();
        }

        public static IReadOnlyList<string> Arguments(VideoInfo info, string sourcePath, string outputPath) =>
            Build(
                $"{info.Width.ToString(CultureInfo.InvariantCulture)}x{info.Height.ToString(CultureInfo.InvariantCulture)}",
                $"{info.FrameRateNumerator.ToString(CultureInfo.InvariantCulture)}/{info.FrameRateDenominator.ToString(CultureInfo.InvariantCulture)}",
                info.AudioTracks.Count, sourcePath, outputPath, VideoLayout.AudioLanguages);

        private static string[] Build(string size, string rate, int audioTracks, string sourcePath, string outputPath, IReadOnlyList<string> languages) =>
        [
            .. Quiet,
            "-f", "rawvideo", "-pixel_format", "yuv420p", "-video_size", size, "-framerate", rate, "-color_range", "pc", "-colorspace", "smpte170m", "-i", "pipe:0",
            .. StrictInput, "-i", sourcePath,
            "-map", "0:v:0", .. Enumerable.Range(0, audioTracks).SelectMany(track => (string[])["-map", $"1:a:{track}"]),
            .. Range, .. Video, .. Audio,
            .. Enumerable.Range(0, audioTracks).SelectMany(track => (string[])[$"-metadata:s:a:{track}", $"language={VideoLayout.LanguageOfTrack(track, languages)}"]),
            .. Container, outputPath,
        ];

        /// <summary>Reads the finished MP4's audio track <paramref name="track"/> without decoding it, to learn its duration from the progress output.</summary>
        public static IReadOnlyList<string> AudioDurationArguments(string mp4Path, int track) =>
            ["-nostdin", "-v", "error", "-nostats", "-progress", "pipe:1", .. StrictRun, "-f", "mp4", "-i", mp4Path, "-map", $"0:a:{track}", "-c", "copy", "-f", "null", "-"];

        /// <summary>The seconds of output ffmpeg reports in the last <c>out_time_us=</c> line of its progress; null when it reported none.</summary>
        public static double? OutputSeconds(string progress)
        {
            double? seconds = null;
            foreach (string line in progress.Split('\n'))
                if (line.Trim() is var text && text.StartsWith("out_time_us=", StringComparison.Ordinal)
                    && long.TryParse(text.AsSpan("out_time_us=".Length), NumberStyles.None, CultureInfo.InvariantCulture, out long microseconds))
                    seconds = microseconds / 1e6;
            return seconds;
        }

        /// <summary>The number of frames ffmpeg reports having written, from the last <c>frame=</c> line of its progress output; null when it reported none.</summary>
        public static int? EncodedFrames(string progress)
        {
            int? frames = null;
            foreach (string line in progress.Split('\n'))
                if (line.Trim() is var text && text.StartsWith("frame=", StringComparison.Ordinal)
                    && int.TryParse(text.AsSpan("frame=".Length), NumberStyles.None, CultureInfo.InvariantCulture, out int count))
                    frames = count;
            return frames;
        }
    }
}
