using System.Collections.Concurrent;
using System.Security.Cryptography;
using AssetConverter.Pipeline;
using AssetConverter.Progress;

namespace AssetConverter.Video
{
    /// <summary>
    /// Converts the base game's videos (every <c>.bk2</c> in <c>Data\Video</c>) to MP4 below the video folder, a bounded number at a time
    /// since each conversion is itself multithreaded; every output is published by rename. A video whose stamp (<see cref="ConversionStamp"/>:
    /// this converter's version, a fingerprint of the file and the identity of decoder, encoder and settings) is current is skipped, and the
    /// stamp is written beside the output only after the output was published. Any failure fails the conversion; nothing is skipped silently.
    /// </summary>
    internal sealed class VideoConversion
    {
        /// <summary>Raised whenever the conversion changes its output for the same game files and tools.</summary>
        internal const int ConverterVersion = 1;

        public const string WorkersVariable = "OPENFO4_VIDEO_WORKERS";
        public const int DefaultWorkers = 2;

        private const int FingerprintSampleBytes = 4 << 20;

        private sealed record VideoWork(string Name, string SourcePath, string OutputPath, long Size, ConversionStamp Stamp, string? StaleReason);

        private readonly string _videoOut;
        private readonly VideoEncoder _encode;
        private readonly int _workers;
        private readonly MemoryBudget _memory;
        private readonly IReadOnlyList<VideoWork> _videos;

        private VideoConversion(string videoOut, VideoEncoder encode, int workers, MemoryBudget memory, IReadOnlyList<VideoWork> videos)
        {
            _videoOut = videoOut;
            _encode = encode;
            _workers = workers;
            _memory = memory;
            _videos = videos;
        }

        /// <summary>The number of videos <see cref="Run"/> converts: those that are not current.</summary>
        public int PendingCount => _videos.Count(video => video.StaleReason is not null);

        /// <summary>The workers named by <c>OPENFO4_VIDEO_WORKERS</c>, else <see cref="DefaultWorkers"/>.</summary>
        /// <exception cref="VideoConversionException">The variable is set to something other than a positive number.</exception>
        public static int WorkersFromEnvironment() => ParseWorkers(Environment.GetEnvironmentVariable(WorkersVariable));

        internal static int ParseWorkers(string? setting) =>
            string.IsNullOrWhiteSpace(setting) ? DefaultWorkers
            : int.TryParse(setting, out int workers) && workers > 0 ? workers
            : throw new VideoConversionException($"{WorkersVariable} is '{setting}', not a positive number.");

        /// <summary>Lists the game's videos and decides which are current.</summary>
        /// <param name="toolVersion">Identifies decoder, encoder and settings, so an output of other tools or settings is converted again.</param>
        /// <param name="force">Converts every video again even when its stamp is current.</param>
        /// <param name="memory">Admits each video's conversion against the install's memory.</param>
        /// <exception cref="VideoConversionException">The game has no video folder or no video in it, or one cannot be read.</exception>
        public static VideoConversion Plan(string gameVideoDirectory, string videoOut, VideoEncoder encode, string toolVersion, MemoryBudget memory, int? workers = null, bool force = false)
        {
            string[] sources = ListVideos(gameVideoDirectory);
            var work = sources.Select(source => Prepare(source, videoOut, toolVersion, force)).ToList();
            return new VideoConversion(videoOut, encode, workers ?? WorkersFromEnvironment(), memory, work);
        }

        private static string[] ListVideos(string gameVideoDirectory)
        {
            if (!Directory.Exists(gameVideoDirectory))
                throw new VideoConversionException($"The video folder '{gameVideoDirectory}' was not found — the Fallout 4 install is incomplete.");
            string[] sources =
            [
                .. Directory.EnumerateFiles(gameVideoDirectory)
                    .Where(path => Path.GetExtension(path).Equals(VideoLayout.SourceExtension, StringComparison.OrdinalIgnoreCase))
                    .Order(StringComparer.OrdinalIgnoreCase),
            ];
            if (sources.Length == 0)
                throw new VideoConversionException($"The video folder '{gameVideoDirectory}' holds no {VideoLayout.SourceExtension} file — the Fallout 4 install is incomplete.");
            return sources;
        }

        private static VideoWork Prepare(string source, string videoOut, string toolVersion, bool force)
        {
            string output = VideoLayout.OutputPath(videoOut, source);
            SourceFingerprint fingerprint = Fingerprint(source);
            var stamp = new ConversionStamp(ConverterVersion, null, [fingerprint], FfmpegVersion: toolVersion);
            string? reason = force ? "a full conversion was requested" : stamp.SidecarStaleReason(output);
            return new VideoWork(Path.GetFileName(source), source, output, fingerprint.Size, stamp, reason);
        }

        /// <summary>
        /// The file's name and size and a hash of its first and last <see cref="FingerprintSampleBytes"/>: videos are gigabytes and
        /// reading none of the middle keeps planning cheap, while a game update changes the size or the frame index at the start or the end.
        /// </summary>
        private static SourceFingerprint Fingerprint(string path)
        {
            try
            {
                using var file = File.OpenRead(path);
                long length = file.Length;
                int head = (int)Math.Min(length, FingerprintSampleBytes);
                int tail = (int)Math.Min(length - head, FingerprintSampleBytes);
                byte[] sample = new byte[head + tail];
                file.ReadExactly(sample.AsSpan(0, head));
                file.Seek(length - tail, SeekOrigin.Begin);
                file.ReadExactly(sample.AsSpan(head, tail));
                return new SourceFingerprint(Path.GetFileName(path), length, Convert.ToHexString(SHA256.HashData(sample)).ToLowerInvariant());
            }
            catch (Exception ex) when (ex is IOException or UnauthorizedAccessException)
            {
                throw new VideoConversionException($"The video '{path}' could not be read: {ex.Message}", ex);
            }
        }

        /// <exception cref="VideoConversionException">A video could not be converted or a stale file could not be removed.</exception>
        public void Run(int progressPhase, CancellationToken ct)
        {
            Directory.CreateDirectory(_videoOut);
            if (PendingCount > 0) RemoveOrphans();
            foreach (VideoWork video in _videos.Where(video => video.StaleReason is null))
                Console.WriteLine($"Video {video.Name} already converted in {_videoOut}; skipping.");

            using var stop = CancellationTokenSource.CreateLinkedTokenSource(ct);
            var failure = new FirstFailure<VideoConversionException>(stop, (message, inner) => new VideoConversionException(message, inner), "videos");
            var pending = _videos.Where(video => video.StaleReason is not null).OrderByDescending(video => video.Size).ToList();
            Parallel.ForEach(
                Partitioner.Create(pending, EnumerablePartitionerOptions.NoBuffering),
                new ParallelOptions { MaxDegreeOfParallelism = _workers },
                video => ConvertOne(video, progressPhase, failure, stop.Token));
            failure.ThrowIfAny();
            ct.ThrowIfCancellationRequested();
        }

        private void ConvertOne(VideoWork video, int progressPhase, FirstFailure<VideoConversionException> failure, CancellationToken stopToken)
        {
            if (stopToken.IsCancellationRequested) return;
            try
            {
                using var memory = _memory.Reserve(MemoryEstimates.VideoConversionBytes, stopToken);
                using var covered = memory.CoverChildProcesses();
                Console.WriteLine($"Video {video.Name} is converted: {video.StaleReason}.");
                InstallProgress.SetDetail(progressPhase, video.Name);
                File.Delete(ConversionStamp.SidecarOf(video.OutputPath));
                AtomicPublish.Produced(video.OutputPath, temporary => _encode(video.SourcePath, temporary, stopToken));
                video.Stamp.WriteSidecar(video.OutputPath);
                InstallProgress.Tick(progressPhase);
            }
            catch (OperationCanceledException) when (stopToken.IsCancellationRequested)
            {
            }
            catch (VideoConversionException ex)
            {
                failure.Record(new VideoConversionException($"Video '{video.Name}' could not be converted: {ex.Message}", ex.InnerException));
            }
            catch (Exception ex)
            {
                failure.Record(new VideoConversionException($"Video '{video.Name}' could not be converted to '{InstallLayout.VideoFolder}/{Path.GetFileName(video.OutputPath)}': {ex.Message}", ex));
            }
        }

        /// <summary>
        /// Removes what an earlier conversion left that no video produces any more: MP4 files and stamps of videos a game update dropped, and
        /// temporary files of an interrupted run. Done before converting, and only when something is converted.
        /// </summary>
        private void RemoveOrphans()
        {
            var produced = _videos.Select(video => Path.GetFullPath(video.OutputPath)).ToHashSet(StringComparer.OrdinalIgnoreCase);
            foreach (string file in Directory.EnumerateFiles(_videoOut))
            {
                string output = file.EndsWith(ConversionStamp.SidecarSuffix, StringComparison.OrdinalIgnoreCase) ? file[..^ConversionStamp.SidecarSuffix.Length] : file;
                bool leftover = file.EndsWith(VideoLayout.Mp4Extension + AtomicPublish.TemporarySuffix, StringComparison.OrdinalIgnoreCase)
                    || file.EndsWith(VideoLayout.Mp4Extension + ConversionStamp.SidecarSuffix + AtomicPublish.TemporarySuffix, StringComparison.OrdinalIgnoreCase);
                bool orphan = output.EndsWith(VideoLayout.Mp4Extension, StringComparison.OrdinalIgnoreCase) && !produced.Contains(Path.GetFullPath(output));
                if (!leftover && !orphan) continue;
                try
                {
                    File.Delete(file);
                }
                catch (Exception ex) when (ex is IOException or UnauthorizedAccessException)
                {
                    throw new VideoConversionException($"Could not remove the stale video file '{file}': {ex.Message}", ex);
                }
            }
        }
    }
}
