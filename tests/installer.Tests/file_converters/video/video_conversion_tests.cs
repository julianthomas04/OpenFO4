using System.Collections.Concurrent;
using AssetConverter;
using AssetConverter.Pipeline;
using AssetConverter.Video;
using InstallerTests.Archives;
using Xunit;

namespace InstallerTests.Video
{
    /// <summary>Prints to the console and reports to the process-wide InstallProgress, so it runs serially with the other tests that do.</summary>
    [Collection("Console redirection")]
    public sealed class VideoConversionTests : IDisposable
    {
        private const string ToolV1 = "tools-1";

        private static CancellationToken Ct => TestContext.Current.CancellationToken;

        private readonly ArchiveTempDir _game = new();
        private readonly ArchiveTempDir _install = new();
        private readonly ConcurrentQueue<string> _encoded = new();

        public void Dispose()
        {
            _game.Dispose();
            _install.Dispose();
        }

        private string VideoOut => Path.Combine(_install.Path, "video");

        private void WriteVideos()
        {
            _game.Write("MainMenuLoop.bk2", TestBytes.Random(500, 1));
            _game.Write("GameIntro_V3_B.BK2", TestBytes.Random(900, 2));
            _game.Write("readme.txt", [1]);
        }

        private VideoEncoder Encoder(Func<string, bool>? fails = null) => (source, output, _) =>
        {
            string name = Path.GetFileName(source);
            _encoded.Enqueue(name);
            if (fails?.Invoke(name) == true) throw new VideoConversionException("The Bink2 decoder exited 1. unsupported feature");
            File.WriteAllBytes(output, [.. "MP4"u8, .. File.ReadAllBytes(source)]);
        };

        private int Run(VideoEncoder encoder, string toolVersion = ToolV1, bool force = false)
        {
            var conversion = VideoConversion.Plan(_game.Path, VideoOut, encoder, toolVersion, MemoryBudget.Unlimited(), workers: 2, force: force);
            int pending = conversion.PendingCount;
            conversion.Run(-1, Ct);
            return pending;
        }

        private string[] Encoded() => [.. _encoded.Order(StringComparer.Ordinal)];

        [Fact]
        public void EveryBk2OfTheFolderBecomesItsLowercasedMp4WithAStamp()
        {
            WriteVideos();

            int pending = Run(Encoder());

            Assert.Equal(2, pending);
            Assert.Equal(["GameIntro_V3_B.BK2", "MainMenuLoop.bk2"], Encoded());
            Assert.Equal([.. "MP4"u8, .. TestBytes.Random(500, 1)], File.ReadAllBytes(Path.Combine(VideoOut, "mainmenuloop.mp4")));
            Assert.True(File.Exists(Path.Combine(VideoOut, "gameintro_v3_b.mp4")));
            Assert.True(File.Exists(Path.Combine(VideoOut, "mainmenuloop.mp4.stamp")));
            Assert.Empty(Directory.GetFiles(VideoOut, "*.tmp"));
        }

        [Fact]
        public void CurrentVideosAreSkippedAndEverythingIsConvertedAgainOnForce()
        {
            WriteVideos();
            Run(Encoder());
            _encoded.Clear();

            Assert.Equal(0, Run(Encoder()));
            Assert.Empty(_encoded);
            Assert.Equal(2, Run(Encoder(), force: true));
            Assert.Equal(2, _encoded.Count);
        }

        [Fact]
        public void ADifferentToolVersionConvertsEveryVideoAgain()
        {
            WriteVideos();
            Run(Encoder());
            _encoded.Clear();

            Assert.Equal(2, Run(Encoder(), "tools-2"));
        }

        [Fact]
        public void AChangedSourceConvertsOnlyThatVideo()
        {
            WriteVideos();
            Run(Encoder());
            _encoded.Clear();
            _game.Write("MainMenuLoop.bk2", TestBytes.Random(500, 9));

            Assert.Equal(1, Run(Encoder()));
            Assert.Equal(["MainMenuLoop.bk2"], Encoded());
        }

        [Fact]
        public void AMissingOutputIsConvertedAgainEvenWithItsStamp()
        {
            WriteVideos();
            Run(Encoder());
            _encoded.Clear();
            File.Delete(Path.Combine(VideoOut, "gameintro_v3_b.mp4"));

            Assert.Equal(1, Run(Encoder()));
            Assert.Equal(["GameIntro_V3_B.BK2"], Encoded());
        }

        [Fact]
        public void AFailingVideoFailsTheConversionNamingItWithTheToolsMessageAndLeavesNoOutputOrStamp()
        {
            WriteVideos();

            var error = Assert.Throws<VideoConversionException>(() => Run(Encoder(name => name.StartsWith("GameIntro", StringComparison.Ordinal))));

            Assert.Contains("GameIntro_V3_B.BK2", error.Message, StringComparison.Ordinal);
            Assert.Contains("unsupported feature", error.Message, StringComparison.Ordinal);
            Assert.False(File.Exists(Path.Combine(VideoOut, "gameintro_v3_b.mp4")));
            Assert.False(File.Exists(Path.Combine(VideoOut, "gameintro_v3_b.mp4.stamp")));
            Assert.Empty(Directory.GetFiles(VideoOut, "*.tmp"));
        }

        [Fact]
        public void AFailureOfAnotherKindIsWrappedWithTheVideoAndTheOutputItWasMeantFor()
        {
            WriteVideos();
            VideoEncoder broken = (_, _, _) => throw new IOException("disk full");

            var error = Assert.Throws<VideoConversionException>(() => Run(broken));

            Assert.Contains("disk full", error.Message, StringComparison.Ordinal);
            Assert.Contains("video/", error.Message, StringComparison.Ordinal);
        }

        [Fact]
        public void OrphansAndLeftoversAreRemovedBeforeConvertingOnlyWhenSomethingIsConverted()
        {
            WriteVideos();
            Directory.CreateDirectory(VideoOut);
            string dropped = Path.Combine(VideoOut, "dropped.mp4");
            string droppedStamp = dropped + ".stamp";
            string leftover = Path.Combine(VideoOut, "gameintro_v3_b.mp4.tmp");
            string stampLeftover = Path.Combine(VideoOut, "gameintro_v3_b.mp4.stamp.tmp");
            File.WriteAllBytes(stampLeftover, [1]);
            File.WriteAllBytes(dropped, [1]);
            File.WriteAllBytes(droppedStamp, [1]);
            File.WriteAllBytes(leftover, [1]);

            Run(Encoder());

            Assert.False(File.Exists(dropped));
            Assert.False(File.Exists(droppedStamp));
            Assert.False(File.Exists(leftover));
            Assert.False(File.Exists(stampLeftover));
            File.WriteAllBytes(dropped, [1]);
            Run(Encoder());
            Assert.True(File.Exists(dropped));
        }

        [Fact]
        public void AMissingVideoFolderIsAHardError()
        {
            var error = Assert.Throws<VideoConversionException>(() => VideoConversion.Plan(Path.Combine(_game.Path, "nowhere"), VideoOut, Encoder(), ToolV1, MemoryBudget.Unlimited(), workers: 1));

            Assert.Contains("nowhere", error.Message, StringComparison.Ordinal);
        }

        [Fact]
        public void AFolderWithoutAnyBk2IsAHardError()
        {
            _game.Write("readme.txt", [1]);

            var error = Assert.Throws<VideoConversionException>(() => VideoConversion.Plan(_game.Path, VideoOut, Encoder(), ToolV1, MemoryBudget.Unlimited(), workers: 1));

            Assert.Contains(".bk2", error.Message, StringComparison.Ordinal);
        }

        [Fact]
        public void SeveralFailuresAreReportedAsTheFirstWithTheCountOfTheOthers()
        {
            _game.Write("a.bk2", [1]);
            _game.Write("b.bk2", [2]);
            using var bothStarted = new Barrier(2);
            VideoEncoder failing = (_, _, _) =>
            {
                bothStarted.SignalAndWait(TimeSpan.FromSeconds(10), Ct);
                throw new VideoConversionException("bad stream");
            };

            var error = Assert.Throws<VideoConversionException>(() => Run(failing));

            Assert.Contains("bad stream", error.Message, StringComparison.Ordinal);
            Assert.Contains("(1 more videos failed)", error.Message, StringComparison.Ordinal);
        }

        [Fact]
        public void AtMostTheConfiguredNumberOfVideosAreConvertedAtOnce()
        {
            for (int i = 0; i < 5; i++) _game.Write($"v{i}.bk2", [(byte)i]);
            int running = 0, peak = 0;
            VideoEncoder counting = (_, output, _) =>
            {
                int now = Interlocked.Increment(ref running);
                int seen;
                while ((seen = Volatile.Read(ref peak)) < now && Interlocked.CompareExchange(ref peak, now, seen) != seen) { }
                SpinWait.SpinUntil(() => Volatile.Read(ref peak) >= 2, TimeSpan.FromSeconds(10));
                Interlocked.Decrement(ref running);
                File.WriteAllBytes(output, [1]);
            };

            Run(counting);

            Assert.Equal(2, peak);
        }

        [Fact]
        public void CancellingStopsTheConversionWithACancellation()
        {
            WriteVideos();
            using var cancel = new CancellationTokenSource();
            VideoEncoder cancelling = (_, _, token) =>
            {
                cancel.Cancel();
                throw new OperationCanceledException(token);
            };
            var conversion = VideoConversion.Plan(_game.Path, VideoOut, cancelling, ToolV1, MemoryBudget.Unlimited(), workers: 2);

            Assert.ThrowsAny<OperationCanceledException>(() => conversion.Run(-1, cancel.Token));
        }

        [Fact]
        public void EachVideoHoldsItsEstimateFromTheBudgetWhileItConvertsAndReleasesItAfterwards()
        {
            WriteVideos();
            var budget = new MemoryBudget(2 * MemoryEstimates.VideoConversionBytes);
            var reservedDuringConversion = new ConcurrentBag<long>();
            VideoEncoder encoder = (source, output, _) =>
            {
                reservedDuringConversion.Add(budget.ReservedBytes);
                File.WriteAllBytes(output, [1]);
            };

            VideoConversion.Plan(_game.Path, VideoOut, encoder, ToolV1, workers: 1, memory: budget).Run(-1, Ct);

            Assert.All(reservedDuringConversion, reserved => Assert.Equal(MemoryEstimates.VideoConversionBytes, reserved));
            Assert.Equal(2, reservedDuringConversion.Count);
            Assert.Equal(0, budget.ReservedBytes);
        }

        [Fact]
        public async Task AVideoWaitsForTheBudgetWhileAnotherOneConverts()
        {
            WriteVideos();
            var budget = new MemoryBudget(MemoryEstimates.VideoConversionBytes);
            using var firstStarted = new ManualResetEventSlim();
            using var releaseFirst = new ManualResetEventSlim();
            VideoEncoder encoder = (source, output, _) =>
            {
                _encoded.Enqueue(Path.GetFileName(source));
                if (_encoded.Count == 1)
                {
                    firstStarted.Set();
                    releaseFirst.Wait(Ct);
                }
                File.WriteAllBytes(output, [1]);
            };
            var run = Task.Run(() => VideoConversion.Plan(_game.Path, VideoOut, encoder, ToolV1, workers: 2, memory: budget).Run(-1, Ct), Ct);

            firstStarted.Wait(Ct);
            Assert.True(SpinWait.SpinUntil(() => budget.WaitingCount == 1, TimeSpan.FromSeconds(30)));
            Assert.Single(_encoded);
            releaseFirst.Set();
            await run.WaitAsync(TimeSpan.FromSeconds(30), Ct);

            Assert.Equal(2, _encoded.Count);
            Assert.Equal(0, budget.ReservedBytes);
        }

        [Fact]
        public void AFailedConversionReleasesItsReservation()
        {
            WriteVideos();
            var budget = new MemoryBudget(MemoryEstimates.VideoConversionBytes);

            var conversion = VideoConversion.Plan(_game.Path, VideoOut, Encoder(_ => true), ToolV1, workers: 1, memory: budget);

            Assert.Throws<VideoConversionException>(() => conversion.Run(-1, Ct));
            Assert.Equal(0, budget.ReservedBytes);
        }

        [Theory]
        [InlineData(null, 2)]
        [InlineData("  ", 2)]
        [InlineData("5", 5)]
        public void TheWorkersComeFromTheEnvironmentOrDefaultToTwo(string? setting, int expected)
        {
            Assert.Equal(expected, VideoConversion.ParseWorkers(setting));
        }

        [Theory]
        [InlineData("0")]
        [InlineData("-1")]
        [InlineData("many")]
        public void ANonPositiveWorkerCountFailsExplicitly(string setting)
        {
            Assert.Throws<VideoConversionException>(() => VideoConversion.ParseWorkers(setting));
        }
    }
}
