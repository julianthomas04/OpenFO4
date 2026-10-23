using AssetConverter.Audio;

namespace AssetConverter.Video
{
    /// <summary>The conversion of the video at <paramref name="sourcePath"/> to the MP4 at <paramref name="outputPath"/>.</summary>
    internal delegate void VideoEncoder(string sourcePath, string outputPath, CancellationToken ct);

    /// <summary>
    /// Converts one video with two processes joined by a pipe: the decoder's raw frames stream from its stdout into the stdin of ffmpeg
    /// (about 8 GB per video, never held in memory). Either process failing fails the video with the texts of both; the video must
    /// come out with exactly the frames the decoder announced, counted at the pipe and again by ffmpeg.
    /// </summary>
    internal static class VideoTranscoding
    {
        private const int CopyBufferBytes = 1 << 20;

        public static VideoEncoder With(Bink2Decoder decoder, Ffmpeg ffmpeg) =>
            (sourcePath, outputPath, ct) => Transcode(decoder, ffmpeg, sourcePath, outputPath, ct);

        private static void Transcode(Bink2Decoder decoder, Ffmpeg ffmpeg, string sourcePath, string outputPath, CancellationToken ct)
        {
            VideoInfo info = decoder.Info(sourcePath, ct);
            long piped = 0;
            string progress = ffmpeg.RunStreaming(
                VideoEncoding.Arguments(info, sourcePath, outputPath),
                (stdin, token) => piped = Pipe(decoder, sourcePath, info, stdin, token),
                ct);
            VerifyFrames(sourcePath, info, piped, VideoEncoding.EncodedFrames(progress));
            VerifyAudioDurations(ffmpeg, sourcePath, outputPath, info, ct);
        }

        /// <summary>
        /// Every audio track of the MP4 must last as long as the video, within <see cref="AudioDurationToleranceSeconds"/>: the track's duration
        /// is what ffmpeg reports for copying it to nowhere, so no audio is decoded again. A shorter or longer track would play out of sync or be cut.
        /// </summary>
        private static void VerifyAudioDurations(Ffmpeg ffmpeg, string sourcePath, string outputPath, VideoInfo info, CancellationToken ct)
        {
            double videoSeconds = (double)info.FrameCount * info.FrameRateDenominator / info.FrameRateNumerator;
            for (int track = 0; track < info.AudioTracks.Count; track++)
            {
                double? audioSeconds = VideoEncoding.OutputSeconds(ffmpeg.Capture(VideoEncoding.AudioDurationArguments(outputPath, track), ct));
                if (audioSeconds is not { } measured)
                    throw new VideoConversionException($"The duration of audio track {track} of the conversion of '{sourcePath}' could not be measured.");
                CheckAudioDuration(sourcePath, track, measured, videoSeconds);
            }
        }

        internal const double AudioDurationToleranceSeconds = 0.25;

        internal static void CheckAudioDuration(string sourcePath, int track, double audioSeconds, double videoSeconds)
        {
            if (Math.Abs(audioSeconds - videoSeconds) > AudioDurationToleranceSeconds)
                throw new VideoConversionException(
                    $"Audio track {track} of '{sourcePath}' lasts {audioSeconds:F3} s but its video {videoSeconds:F3} s (more than {AudioDurationToleranceSeconds} s apart).");
        }

        /// <returns>The bytes the decoder delivered.</returns>
        /// <exception cref="VideoConversionException">The decoder failed or delivered other than its announced frames.</exception>
        private static long Pipe(Bink2Decoder decoder, string sourcePath, VideoInfo info, Stream destination, CancellationToken ct)
        {
            using var decoding = decoder.Decode(sourcePath, info.FrameCount);
            using var kill = ct.Register(decoding.Kill);
            byte[] buffer = new byte[CopyBufferBytes];
            long total = 0;
            while (true)
            {
                int read = ReadFromDecoder(decoding, buffer, sourcePath, ct);
                if (read == 0) break;
                try
                {
                    destination.Write(buffer, 0, read);
                }
                catch (Exception ex) when (Ffmpeg.IsClosedInput(ex))
                {
                    decoding.Kill();
                    ct.ThrowIfCancellationRequested();
                    throw new VideoConversionException($"ffmpeg stopped reading its input of '{sourcePath}' after {total:N0} bytes: {ex.Message}", ex);
                }
                total += read;
            }
            ct.ThrowIfCancellationRequested();
            decoding.EnsureSucceeded();
            return total;
        }

        private static int ReadFromDecoder(Bink2Decoder.Decoding decoding, byte[] buffer, string sourcePath, CancellationToken ct)
        {
            try
            {
                return decoding.Output.Read(buffer);
            }
            catch (Exception ex) when (ex is IOException or ObjectDisposedException)
            {
                ct.ThrowIfCancellationRequested();
                throw new VideoConversionException($"Reading the frames of '{sourcePath}' from the Bink2 decoder failed: {ex.Message}", ex);
            }
        }

        internal static void VerifyFrames(string sourcePath, VideoInfo info, long pipedBytes, int? encodedFrames)
        {
            long expectedBytes = info.FrameBytes * info.FrameCount;
            if (pipedBytes != expectedBytes)
                throw new VideoConversionException($"ffmpeg was fed {pipedBytes:N0} bytes of '{sourcePath}', not the {expectedBytes:N0} of its {info.FrameCount:N0} frames.");
            if (encodedFrames != info.FrameCount)
                throw new VideoConversionException($"ffmpeg wrote {(encodedFrames is { } frames ? frames.ToString("N0", System.Globalization.CultureInfo.InvariantCulture) : "an unknown number of")} frames of '{sourcePath}', not the {info.FrameCount:N0} the decoder announced.");
        }
    }
}
