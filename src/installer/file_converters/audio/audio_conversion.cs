using System.Security.Cryptography;
using System.Text;
using System.Threading.Channels;
using AssetConverter.Pipeline;
using AssetConverter.Progress;
using SharpBSABA2;
using SharpBSABA2.BA2Util;

namespace AssetConverter.Audio
{
    /// <summary>Writes the conversion of <paramref name="entry"/>, whose bytes are <paramref name="content"/>, to <paramref name="outputPath"/>.</summary>
    internal delegate void AudioEncoder(AudioEntry entry, byte[] content, string outputPath, CancellationToken ct);

    /// <summary>
    /// Converts the base game's sound and music (<see cref="AudioLayout.Archives"/>) to FLAC below the audio folder. Each archive is read
    /// sequentially by one reader that inflates entries into memory (bounded by an <see cref="InFlightBudget"/>) while a pool of workers
    /// runs one encoder process per entry; every output is published by rename. An archive whose marker records the current
    /// <see cref="ConversionStamp"/> (this converter's version, the encoder tool and the archive's entries) is skipped, and the marker
    /// is written only after every entry of the archive was converted. Any failure fails the conversion; nothing is skipped silently.
    /// </summary>
    internal sealed class AudioConversion : IDisposable
    {
        /// <summary>Raised whenever the conversion changes its output for the same game files and encoder.</summary>
        internal const int ConverterVersion = 1;

        public static int DefaultWorkers => Math.Max(1, Environment.ProcessorCount / 2);

        private sealed record ArchiveWork(string Name, BA2 Archive, IReadOnlyList<AudioEntry> Entries, IReadOnlyDictionary<string, ArchiveEntry> Sources, ConversionStamp Stamp)
        {
            public string? StaleReason { get; init; }
        }

        private sealed record Job(AudioEntry Entry, byte[] Content, long BudgetBytes);

        private readonly string _audioOut;
        private readonly AudioEncoder _encode;
        private readonly int _workers;
        private readonly MemoryBudget _memory;
        private readonly List<ArchiveWork> _archives;

        private AudioConversion(string audioOut, AudioEncoder encode, int workers, MemoryBudget memory, List<ArchiveWork> archives)
        {
            _audioOut = audioOut;
            _encode = encode;
            _workers = workers;
            _memory = memory;
            _archives = archives;
        }

        /// <summary>The number of files <see cref="Run"/> converts: the entries of the archives that are not current.</summary>
        public int PendingFileCount => _archives.Where(archive => archive.StaleReason is not null).Sum(archive => archive.Entries.Count);

        /// <summary>Fails when a source archive of the game is missing, so a caller can find out before it deletes anything.</summary>
        /// <exception cref="AudioConversionException">An archive is missing.</exception>
        public static void RequireArchives(string gameDataDirectory)
        {
            string[] missing = [.. AudioLayout.Archives.Select(name => Path.Combine(gameDataDirectory, name)).Where(path => !File.Exists(path))];
            if (missing.Length > 0)
                throw new AudioConversionException($"Audio archive(s) not found: {string.Join(", ", missing.Select(path => $"'{path}'"))} — the Fallout 4 install is incomplete.");
        }

        /// <summary>Opens the game's audio archives, plans what to convert and decides which archives are current. Dispose to close the archives.</summary>
        /// <param name="toolVersion">Identifies the encoder, so an output of another encoder is converted again.</param>
        /// <exception cref="AudioConversionException">An archive is missing or unreadable, or the entries cannot be converted as planned.</exception>
        /// <param name="force">Converts every archive again even when its marker is current.</param>
        /// <param name="memory">Admits each entry's conversion against the install's memory.</param>
        public static AudioConversion Plan(string gameDataDirectory, string audioOut, AudioEncoder encode, string toolVersion, MemoryBudget memory, int? workers = null, bool force = false)
        {
            RequireArchives(gameDataDirectory);
            var opened = new List<(string Name, BA2 Archive)>();
            try
            {
                foreach (string name in AudioLayout.Archives)
                    opened.Add((name, Open(Path.Combine(gameDataDirectory, name))));
                var plan = AudioPlan.Build([.. opened.Select(archive => archive.Archive.Files.Select(file => file.FullPath))]);
                var work = opened.Select((archive, index) => PrepareArchive(archive.Name, archive.Archive, plan[index], audioOut, toolVersion, force)).ToList();
                return new AudioConversion(audioOut, encode, workers ?? DefaultWorkers, memory, work);
            }
            catch
            {
                foreach (var (_, archive) in opened) archive.Close();
                throw;
            }
        }

        private static BA2 Open(string path)
        {
            try
            {
                return new BA2(path);
            }
            catch (Exception ex) when (ex is IOException or InvalidDataException or UnauthorizedAccessException)
            {
                throw new AudioConversionException($"Failed to open '{path}' (corrupt or locked).", ex);
            }
        }

        private static ArchiveWork PrepareArchive(string name, BA2 archive, IReadOnlyList<AudioEntry> entries, string audioOut, string toolVersion, bool force)
        {
            var sources = new Dictionary<string, ArchiveEntry>(ArchivePath.Comparer);
            foreach (ArchiveEntry file in archive.Files) sources[file.FullPath] = file;
            var stamp = new ConversionStamp(ConverterVersion, null, [Fingerprint(name, archive, entries, sources)], FfmpegVersion: toolVersion);
            string? reason = force ? "a full conversion was requested" : stamp.StaleReasonAt(AudioLayout.MarkerPath(audioOut, name));
            return new ArchiveWork(name, archive, entries, sources, stamp) { StaleReason = reason };
        }

        /// <summary>The archive's file size and a hash of the entries it converts with their sizes; reading no entry content keeps it cheap.</summary>
        private static SourceFingerprint Fingerprint(string name, BA2 archive, IReadOnlyList<AudioEntry> entries, IReadOnlyDictionary<string, ArchiveEntry> sources)
        {
            string listing = string.Concat(entries.OrderBy(entry => entry.Path, StringComparer.Ordinal).Select(entry => $"{entry.Path}\t{sources[entry.Path].RealSize}\n"));
            string hash = Convert.ToHexString(SHA256.HashData(Encoding.UTF8.GetBytes(listing))).ToLowerInvariant();
            return new SourceFingerprint(name, new FileInfo(archive.FullPath).Length, hash);
        }

        /// <exception cref="AudioConversionException">An entry could not be read or converted.</exception>
        public void Run(int progressPhase, CancellationToken ct)
        {
            Directory.CreateDirectory(_audioOut);
            if (PendingFileCount > 0) RemoveOrphans();
            foreach (ArchiveWork archive in _archives)
            {
                if (archive.StaleReason is null)
                {
                    Console.WriteLine($"Audio of {archive.Name} already converted in {_audioOut}; skipping.");
                    continue;
                }
                Console.WriteLine($"Audio of {archive.Name} is converted: {archive.StaleReason}.");
                InstallProgress.SetDetail(progressPhase, archive.Name);
                File.Delete(AudioLayout.MarkerPath(_audioOut, archive.Name));
                ConvertArchive(archive, progressPhase, ct);
                archive.Stamp.WriteTo(AudioLayout.MarkerPath(_audioOut, archive.Name));
            }
        }

        /// <summary>
        /// Removes what an earlier conversion left that no archive entry produces any more: FLAC files of entries a game update dropped
        /// or renamed, and temporary files of an interrupted run. Done before converting, and only when something is converted.
        /// </summary>
        private void RemoveOrphans()
        {
            var produced = _archives.SelectMany(archive => archive.Entries)
                .Select(entry => Path.GetFullPath(AudioLayout.OutputPath(_audioOut, entry.Key)))
                .ToHashSet(StringComparer.OrdinalIgnoreCase);
            foreach (string file in Directory.EnumerateFiles(_audioOut, "*", SearchOption.AllDirectories))
            {
                bool leftover = file.EndsWith(AudioLayout.FlacExtension + AtomicPublish.TemporarySuffix, StringComparison.OrdinalIgnoreCase);
                bool orphan = file.EndsWith(AudioLayout.FlacExtension, StringComparison.OrdinalIgnoreCase) && !produced.Contains(Path.GetFullPath(file));
                if (!leftover && !orphan) continue;
                try
                {
                    File.Delete(file);
                }
                catch (Exception ex) when (ex is IOException or UnauthorizedAccessException)
                {
                    throw new AudioConversionException($"Could not remove the stale audio file '{file}': {ex.Message}", ex);
                }
            }
        }

        private void ConvertArchive(ArchiveWork archive, int progressPhase, CancellationToken ct)
        {
            using var budget = new InFlightBudget(BA2ParallelUnpacker.DefaultMaxInFlightBytes);
            using var stop = CancellationTokenSource.CreateLinkedTokenSource(ct);
            var channel = Channel.CreateUnbounded<Job>(new UnboundedChannelOptions { SingleWriter = true, SingleReader = false });
            var failure = new FirstFailure<AudioConversionException>(stop, (message, inner) => new AudioConversionException(message, inner), "entries");

            var workers = Enumerable.Range(0, _workers).Select(_ => Task.Run(async () =>
            {
                await foreach (Job job in channel.Reader.ReadAllAsync(CancellationToken.None))
                {
                    try
                    {
                        if (!stop.IsCancellationRequested)
                        {
                            ConvertEntry(job.Entry, job.Content, stop.Token);
                            InstallProgress.Tick(progressPhase);
                        }
                    }
                    catch (OperationCanceledException) when (stop.IsCancellationRequested)
                    {
                    }
                    catch (Exception ex)
                    {
                        failure.Record(ex);
                    }
                    finally
                    {
                        budget.Return(job.BudgetBytes);
                    }
                }
            }, CancellationToken.None)).ToArray();

            try
            {
                ReadEntries(archive, channel.Writer, budget, failure, stop.Token);
            }
            finally
            {
                channel.Writer.TryComplete();
                Task.WaitAll(workers, CancellationToken.None);
            }

            failure.ThrowIfAny();
            ct.ThrowIfCancellationRequested();
        }

        /// <summary>Walks the archive's entries in the order their data sits in the file, so the source only ever sees forward reads.</summary>
        private static void ReadEntries(ArchiveWork archive, ChannelWriter<Job> jobs, InFlightBudget budget, FirstFailure<AudioConversionException> failure, CancellationToken ct)
        {
            foreach (AudioEntry entry in archive.Entries.OrderBy(entry => archive.Sources[entry.Path].Offset))
            {
                if (ct.IsCancellationRequested) return;
                ArchiveEntry source = archive.Sources[entry.Path];
                long bytes = source.RealSize;
                try
                {
                    budget.Take(bytes, ct);
                }
                catch (OperationCanceledException)
                {
                    return;
                }
                try
                {
                    using MemoryStream data = source.GetDataStream();
                    jobs.TryWrite(new Job(entry, data.ToArray(), bytes));
                }
                catch (Exception ex) when (ex is not OperationCanceledException)
                {
                    budget.Return(bytes);
                    failure.Record(new AudioConversionException($"Audio entry '{entry.Path}' of '{archive.Name}' could not be read: {ex.Message}", ex));
                    return;
                }
            }
        }

        private void ConvertEntry(AudioEntry entry, byte[] content, CancellationToken ct)
        {
            using var memory = _memory.Reserve(MemoryEstimates.AudioConversionBytes(content.Length), ct);
            using var covered = memory.CoverChildProcesses();
            try
            {
                AtomicPublish.Produced(AudioLayout.OutputPath(_audioOut, entry.Key), temporary => _encode(entry, content, temporary, ct));
            }
            catch (Exception ex) when (ex is not OperationCanceledException and not AudioConversionException)
            {
                throw new AudioConversionException($"Audio entry '{entry.Path}' could not be converted to '{InstallLayout.AudioFolder}/{entry.Key}': {ex.Message}", ex);
            }
        }

        public void Dispose()
        {
            foreach (ArchiveWork archive in _archives) archive.Archive.Close();
        }
    }
}
