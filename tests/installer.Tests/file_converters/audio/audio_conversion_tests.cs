using System.Collections.Concurrent;
using AssetConverter;
using AssetConverter.Audio;
using AssetConverter.Pipeline;
using InstallerTests.Archives;
using Xunit;

namespace InstallerTests.Audio
{
    /// <summary>Prints to the console and reports to the process-wide InstallProgress, so it runs serially with the other tests that do.</summary>
    [Collection("Console redirection")]
    public sealed class AudioConversionTests : IDisposable
    {
        private const string Sounds = "Fallout4 - Sounds.ba2";
        private const string Startup = "Fallout4 - Startup.ba2";
        private const string ToolV1 = "ffmpeg-1";

        private static CancellationToken Ct => TestContext.Current.CancellationToken;

        private readonly ArchiveTempDir _game = new();
        private readonly ArchiveTempDir _install = new();
        private readonly ConcurrentQueue<string> _encoded = new();

        public void Dispose()
        {
            _game.Dispose();
            _install.Dispose();
        }

        private string AudioOut => Path.Combine(_install.Path, "audio");

        private static Ba2File Entry(string path, int length, int seed, bool compress = false) => new(path, TestBytes.Random(length, seed), compress);

        private void WriteArchive(string name, params Ba2File[] files) => _game.Write(name, Ba2Builder.Gnrl(files));

        private void WriteDefaultArchives()
        {
            WriteArchive(Sounds, Entry(@"Sound\fx\boom.xwm", 300, 1), Entry(@"Sound\fx\Click.wav", 200, 2, compress: true), Entry(@"Strings\a.strings", 10, 3));
            WriteArchive(Startup, Entry(@"Music\Special\MUS_MainTheme.xwm", 400, 4), Entry(@"Sound\voice\npc\line.fuz", 50, 5));
        }

        /// <summary>Stands in for ffmpeg: writes the entry's bytes behind a marker and remembers which entries it was asked for.</summary>
        private AudioEncoder Encoder(Func<AudioEntry, bool>? fails = null) => (entry, content, outputPath, _) =>
        {
            _encoded.Enqueue(entry.Path);
            if (fails?.Invoke(entry) == true) throw new FfmpegException("ffmpeg exited 1. Invalid data found when processing input");
            File.WriteAllBytes(outputPath, [.. "FLAC"u8, .. content]);
        };

        private int Run(AudioEncoder encoder, string toolVersion = ToolV1)
        {
            using var conversion = AudioConversion.Plan(_game.Path, AudioOut, encoder, toolVersion, MemoryBudget.Unlimited(), workers: 3);
            int pending = conversion.PendingFileCount;
            conversion.Run(-1, Ct);
            return pending;
        }

        private string[] EncodedEntries() => [.. _encoded.Order(StringComparer.Ordinal)];

        [Fact]
        public void EverySoundAndMusicEntryOfBothArchivesBecomesItsFlacFile()
        {
            WriteDefaultArchives();

            int pending = Run(Encoder());

            Assert.Equal(3, pending);
            Assert.Equal([@"Music\Special\MUS_MainTheme.xwm", @"Sound\fx\Click.wav", @"Sound\fx\boom.xwm"], EncodedEntries());
            Assert.Equal([.. "FLAC"u8, .. TestBytes.Random(300, 1)], File.ReadAllBytes(Path.Combine(AudioOut, "sound", "fx", "boom.flac")));
            Assert.Equal([.. "FLAC"u8, .. TestBytes.Random(200, 2)], File.ReadAllBytes(Path.Combine(AudioOut, "sound", "fx", "click.flac")));
            Assert.True(File.Exists(Path.Combine(AudioOut, "music", "special", "mus_maintheme.flac")));
            Assert.Equal(3, Directory.GetFiles(AudioOut, "*.flac", SearchOption.AllDirectories).Length);
            Assert.True(File.Exists(AudioLayout.MarkerPath(AudioOut, Sounds)));
            Assert.True(File.Exists(AudioLayout.MarkerPath(AudioOut, Startup)));
        }

        [Fact]
        public void APathInBothArchivesIsConvertedOnceFromTheLaterArchive()
        {
            WriteArchive(Sounds, Entry(@"Sound\fx\ui_ok.xwm", 100, 1), Entry(@"Sound\fx\only_sounds.xwm", 100, 2));
            WriteArchive(Startup, Entry(@"Sound\FX\UI_OK.xwm", 120, 3));

            int pending = Run(Encoder());

            Assert.Equal(2, pending);
            Assert.Equal([@"Sound\FX\UI_OK.xwm", @"Sound\fx\only_sounds.xwm"], EncodedEntries());
            Assert.Equal([.. "FLAC"u8, .. TestBytes.Random(120, 3)], File.ReadAllBytes(Path.Combine(AudioOut, "sound", "fx", "ui_ok.flac")));
        }

        [Fact]
        public void ArchivesWithTheCurrentMarkerAreSkippedOnTheNextRun()
        {
            WriteDefaultArchives();
            Run(Encoder());
            _encoded.Clear();

            int pending = Run(Encoder());

            Assert.Equal(0, pending);
            Assert.Empty(_encoded);
            Assert.Equal(3, Directory.GetFiles(AudioOut, "*.flac", SearchOption.AllDirectories).Length);
        }

        [Fact]
        public void ANewEncoderVersionConvertsEverythingAgain()
        {
            WriteDefaultArchives();
            Run(Encoder());
            _encoded.Clear();

            int pending = Run(Encoder(), toolVersion: "ffmpeg-2");

            Assert.Equal(3, pending);
            Assert.Equal(3, _encoded.Count);
        }

        [Fact]
        public void OnlyTheArchiveThatChangedIsConvertedAgain()
        {
            WriteDefaultArchives();
            Run(Encoder());
            _encoded.Clear();
            WriteArchive(Startup, Entry(@"Music\Special\MUS_MainTheme.xwm", 410, 4));

            int pending = Run(Encoder());

            Assert.Equal(1, pending);
            Assert.Equal([@"Music\Special\MUS_MainTheme.xwm"], EncodedEntries());
        }

        [Fact]
        public void AFailedEntryFailsTheConversionNamingItAndLeavesNoMarkerOrTemporaryFile()
        {
            WriteDefaultArchives();

            var error = Assert.Throws<AudioConversionException>(() => Run(Encoder(entry => entry.Path == @"Sound\fx\Click.wav")));

            Assert.Contains(@"Sound\fx\Click.wav", error.Message, StringComparison.Ordinal);
            Assert.Contains("Invalid data found", error.Message, StringComparison.Ordinal);
            Assert.IsType<FfmpegException>(error.InnerException);
            Assert.False(File.Exists(AudioLayout.MarkerPath(AudioOut, Sounds)));
            Assert.Empty(Directory.GetFiles(AudioOut, "*.tmp", SearchOption.AllDirectories));
            Assert.False(File.Exists(Path.Combine(AudioOut, "sound", "fx", "click.flac")));
        }

        [Fact]
        public void AfterAFailureTheNextRunConvertsTheIncompleteArchiveInFull()
        {
            WriteDefaultArchives();
            Assert.Throws<AudioConversionException>(() => Run(Encoder(entry => entry.Path == @"Sound\fx\Click.wav")));
            _encoded.Clear();

            Run(Encoder());

            Assert.Contains(@"Sound\fx\boom.xwm", _encoded);
            Assert.Contains(@"Sound\fx\Click.wav", _encoded);
            Assert.True(File.Exists(AudioLayout.MarkerPath(AudioOut, Sounds)));
        }

        [Fact]
        public void ReconvertingAnArchiveRemovesTheFlacOfEntriesItNoLongerHoldsAndTemporaryLeftovers()
        {
            WriteDefaultArchives();
            Run(Encoder());
            string dropped = Path.Combine(AudioOut, "sound", "fx", "boom.flac");
            string leftover = Path.Combine(AudioOut, "sound", "fx", "click.flac.tmp");
            File.WriteAllText(leftover, "half a file");
            WriteArchive(Sounds, Entry(@"Sound\fx\Click.wav", 210, 2, compress: true), Entry(@"Strings\a.strings", 10, 3));

            Run(Encoder());

            Assert.False(File.Exists(dropped));
            Assert.False(File.Exists(leftover));
            Assert.True(File.Exists(Path.Combine(AudioOut, "sound", "fx", "click.flac")));
            Assert.True(File.Exists(Path.Combine(AudioOut, "music", "special", "mus_maintheme.flac")));
            Assert.True(File.Exists(AudioLayout.MarkerPath(AudioOut, Startup)));
        }

        [Fact]
        public void AFullConversionIsRequestedByForce()
        {
            WriteDefaultArchives();
            Run(Encoder());
            _encoded.Clear();
            using var conversion = AudioConversion.Plan(_game.Path, AudioOut, Encoder(), ToolV1, MemoryBudget.Unlimited(), workers: 2, force: true);

            Assert.Equal(3, conversion.PendingFileCount);
            conversion.Run(-1, Ct);

            Assert.Equal(3, _encoded.Count);
        }

        [Fact]
        public void AnInterruptedFullConversionDoesNotLeaveTheMarkerOfTheEarlierRun()
        {
            WriteDefaultArchives();
            Run(Encoder());
            using var conversion = AudioConversion.Plan(_game.Path, AudioOut, Encoder(entry => entry.Path == @"Sound\fx\Click.wav"), ToolV1, MemoryBudget.Unlimited(), workers: 1, force: true);

            Assert.Throws<AudioConversionException>(() => conversion.Run(-1, Ct));

            Assert.False(File.Exists(AudioLayout.MarkerPath(AudioOut, Sounds)));
        }

        [Fact]
        public void APlanThatFailsLeavesTheExistingOutputUntouched()
        {
            WriteDefaultArchives();
            Run(Encoder());
            WriteArchive(Sounds, Entry(@"Sound\fx\boom.wav", 100, 1), Entry(@"Sound\fx\boom.xwm", 100, 2));

            Assert.Throws<AudioConversionException>(() => AudioConversion.Plan(_game.Path, AudioOut, Encoder(), ToolV1, MemoryBudget.Unlimited(), force: true));

            Assert.Equal(3, Directory.GetFiles(AudioOut, "*.flac", SearchOption.AllDirectories).Length);
            Assert.True(File.Exists(AudioLayout.MarkerPath(AudioOut, Sounds)));
        }

        [Fact]
        public void AnEntryThatCannotBeInflatedFailsNamingItWhateverTheReason()
        {
            var sourceArchive = _game.OpenBa2(Sounds, Ba2Builder.Gnrl([new Ba2File(@"Sound\fx\boom.wav", new byte[4000], Compress: true)]));
            long dataOffset = (long)sourceArchive.Files[0].Offset;
            sourceArchive.Close();
            WriteArchive(Startup);
            string sounds = Path.Combine(_game.Path, Sounds);
            byte[] archive = File.ReadAllBytes(sounds);
            for (long offset = dataOffset + 2; offset < dataOffset + 8; offset++) archive[offset] ^= 0xFF;
            File.WriteAllBytes(sounds, archive);

            var error = Assert.Throws<AudioConversionException>(() => Run(Encoder()));

            Assert.Contains(@"Sound\fx\boom.wav", error.Message, StringComparison.Ordinal);
            Assert.NotNull(error.InnerException);
            Assert.Empty(_encoded);
        }

        [Fact]
        public void EntriesWhoseOutputsCoincideFailBeforeAnythingIsConverted()
        {
            WriteArchive(Sounds, Entry(@"Sound\fx\boom.wav", 100, 1), Entry(@"Sound\fx\boom.xwm", 100, 2));
            WriteArchive(Startup);

            var error = Assert.Throws<AudioConversionException>(() => Run(Encoder()));

            Assert.Contains("boom.wav", error.Message, StringComparison.Ordinal);
            Assert.Contains("boom.xwm", error.Message, StringComparison.Ordinal);
            Assert.Empty(_encoded);
        }

        [Fact]
        public void AMissingArchiveFailsNamingItBeforeTheAudioFolderIsTouched()
        {
            WriteArchive(Sounds, Entry(@"Sound\fx\boom.xwm", 100, 1));

            var error = Assert.Throws<AudioConversionException>(() => AudioConversion.RequireArchives(_game.Path));

            Assert.Contains(Startup, error.Message, StringComparison.Ordinal);
            Assert.False(Directory.Exists(AudioOut));
        }

        [Fact]
        public void EachEntryHoldsTheEncoderProcessPlusItsContentFromTheBudgetWhileItConverts()
        {
            WriteDefaultArchives();
            var budget = new MemoryBudget(1L << 40);
            var reservedPerContentLength = new ConcurrentDictionary<int, long>();
            AudioEncoder encoder = (_, content, outputPath, _) =>
            {
                reservedPerContentLength[content.Length] = budget.ReservedBytes;
                File.WriteAllBytes(outputPath, content);
            };

            using (var conversion = AudioConversion.Plan(_game.Path, AudioOut, encoder, ToolV1, workers: 1, memory: budget))
                conversion.Run(-1, Ct);

            Assert.Equal(
                new Dictionary<int, long>
                {
                    [300] = MemoryEstimates.AudioConversionBytes(300),
                    [200] = MemoryEstimates.AudioConversionBytes(200),
                    [400] = MemoryEstimates.AudioConversionBytes(400),
                },
                reservedPerContentLength.ToDictionary(pair => pair.Key, pair => pair.Value));
            Assert.Equal(0, budget.ReservedBytes);
        }

        [Fact]
        public async Task AnEntryWaitsForTheBudgetWhileAnotherOneConverts()
        {
            WriteDefaultArchives();
            var budget = new MemoryBudget(MemoryEstimates.AudioConversionBytes(1000));
            using var firstStarted = new ManualResetEventSlim();
            using var releaseFirst = new ManualResetEventSlim();
            AudioEncoder encoder = (entry, content, outputPath, _) =>
            {
                _encoded.Enqueue(entry.Path);
                if (_encoded.Count == 1)
                {
                    firstStarted.Set();
                    releaseFirst.Wait(Ct);
                }
                File.WriteAllBytes(outputPath, content);
            };
            using var conversion = AudioConversion.Plan(_game.Path, AudioOut, encoder, ToolV1, workers: 2, memory: budget);
            var run = Task.Run(() => conversion.Run(-1, Ct), Ct);

            firstStarted.Wait(Ct);
            Assert.True(SpinWait.SpinUntil(() => budget.WaitingCount == 1, TimeSpan.FromSeconds(30)));
            Assert.Single(_encoded);
            releaseFirst.Set();
            await run.WaitAsync(TimeSpan.FromSeconds(30), Ct);

            Assert.Equal(3, _encoded.Count);
            Assert.Equal(0, budget.ReservedBytes);
        }

        [Fact]
        public void AFailedEntryReleasesItsReservation()
        {
            WriteDefaultArchives();
            var budget = new MemoryBudget(1L << 40);
            using var conversion = AudioConversion.Plan(_game.Path, AudioOut, Encoder(_ => true), ToolV1, workers: 2, memory: budget);

            Assert.Throws<AudioConversionException>(() => conversion.Run(-1, Ct));

            Assert.Equal(0, budget.ReservedBytes);
        }

        [Fact]
        public void CancellingStopsTheConversionWithoutAMarker()
        {
            WriteDefaultArchives();
            using var cancel = CancellationTokenSource.CreateLinkedTokenSource(Ct);
            AudioEncoder cancelling = (_, _, _, token) =>
            {
                cancel.Cancel();
                token.ThrowIfCancellationRequested();
            };
            using var conversion = AudioConversion.Plan(_game.Path, AudioOut, cancelling, ToolV1, MemoryBudget.Unlimited(), workers: 1);

            Assert.ThrowsAny<OperationCanceledException>(() => conversion.Run(-1, cancel.Token));

            Assert.False(File.Exists(AudioLayout.MarkerPath(AudioOut, Sounds)));
        }
    }
}
