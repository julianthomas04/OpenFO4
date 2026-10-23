// SPDX-License-Identifier: GPL-3.0-or-later
using System.Globalization;
using System.Diagnostics;
using System.Security.Cryptography;
using System.Text.Json;

namespace OpenFallout4.Bink2;

internal static class Program
{
    private static readonly JsonSerializerOptions InfoJsonOptions = new() { PropertyNamingPolicy = JsonNamingPolicy.CamelCase };

    internal static string InfoJson(Bink2Container container) => JsonSerializer.Serialize(new
    {
        container.Width, container.Height, container.FrameCount, container.FrameRateNumerator, container.FrameRateDenominator,
        AudioTracks = container.AudioTracks.Select(track => new { track.Id, track.SampleRate, track.Channels }),
    }, InfoJsonOptions);

    public static int Main(string[] args)
    {
        try
        {
            if (args is ["--self-test"]) return SelfTests.Run();
            if (args.Length < 2 || args[0] is not ("info" or "decode"))
                throw new ArgumentException("Usage: info INPUT | decode INPUT --frames N [--start-frame N] [--output PATH] [--hash-output PATH] | --self-test");
            using var container = Bink2Container.Open(args[1]);
            if (args[0] == "info")
            {
                if (args.Length != 2) throw new ArgumentException("info accepts only an input path.");
                Console.WriteLine(InfoJson(container));
                return 0;
            }
            var options = ParseOptions(args[2..]);
            if (!options.TryGetValue("--frames", out var countText)) throw new ArgumentException("--frames is required.");
            var count = ParseNumber(countText);
            var start = options.TryGetValue("--start-frame", out var startText) ? ParseNumber(startText) : 0;
            if (count <= 0 || start < 0 || (long)start + count > container.FrameCount)
                throw new ArgumentException("Requested frame range is outside the container.");
            var deringName = options.GetValueOrDefault("--dering", "all");
            var dering = deringName switch
            {
                "all" => DeringComponents.All,
                "luma" => DeringComponents.Luma,
                "chroma" => DeringComponents.Chroma,
                "none" => DeringComponents.None,
                _ => throw new ArgumentException("--dering must be all, luma, chroma or none."),
            };
            int? deringFrame = options.TryGetValue("--dering-frame", out var frameText) ? ParseNumber(frameText) : null;
            if (deringFrame.HasValue && (deringFrame.Value < 0 || deringFrame.Value >= container.FrameCount))
                throw new ArgumentException("--dering-frame must be a zero-based source frame index.");
            var chromaDcAverageName = options.GetValueOrDefault("--chroma-dc-average", "staged");
            var exactChromaDcAverage = chromaDcAverageName switch
            {
                "staged" => false,
                "exact" => true,
                _ => throw new ArgumentException("--chroma-dc-average must be staged or exact."),
            };
            options.TryGetValue("--output", out var outputPath);
            options.TryGetValue("--hash-output", out var hashPath);
            options.TryGetValue("--dc-average-stats", out var statsPath);
            options.TryGetValue("--block-trace", out var tracePath);
            options.TryGetValue("--trace-mb", out var traceMbText);
            options.TryGetValue("--trace-frames", out var traceFramesText);
            int[] traceMb = [], traceFrames = [];
            if (tracePath is not null || traceMbText is not null || traceFramesText is not null)
            {
                if (tracePath is null || traceMbText is null || traceFramesText is null)
                    throw new ArgumentException("--block-trace, --trace-mb and --trace-frames must be supplied together.");
                if (dering != DeringComponents.All || deringFrame.HasValue || exactChromaDcAverage)
                    throw new ArgumentException("Block tracing requires baseline staged averaging and all deringing.");
                traceMb = traceMbText.Split(',').Select(ParseNumber).ToArray();
                traceFrames = traceFramesText.Split(',').Select(ParseNumber).ToArray();
                if (traceMb.Length != 2 || traceMb.Any(v => v < 0) || traceFrames.Length is < 1 or > 3 ||
                    traceFrames.Distinct().Count() != traceFrames.Length || traceFrames.Any(v => v < 0 || v >= start + count))
                    throw new ArgumentException("Trace requires one macroblock X,Y and one through three distinct decoded source frame indices.");
                if ((long)traceMb[0] * 32 + 16 > container.Width || (long)traceMb[1] * 32 + 16 > container.Height)
                    throw new ArgumentException("Trace ROI must fit the visible image.");
            }
            var binaryStdout = outputPath == "-";
            if (outputPath is null && hashPath is null) throw new ArgumentException("An output or hash-output path is required.");
            if (outputPath is not null && !binaryStdout) RejectSamePath(args[1], outputPath);
            if (hashPath is not null) RejectSamePath(args[1], hashPath);
            if (outputPath is not null && hashPath is not null) RejectSamePath(outputPath, hashPath);
            if (statsPath is not null)
            {
                RejectSamePath(args[1], statsPath);
                if (outputPath is not null && !binaryStdout) RejectSamePath(outputPath, statsPath);
                if (hashPath is not null) RejectSamePath(hashPath, statsPath);
            }
            if (tracePath is not null)
            {
                RejectSamePath(args[1], tracePath);
                if (outputPath is not null && !binaryStdout) RejectSamePath(outputPath, tracePath);
                if (hashPath is not null) RejectSamePath(hashPath, tracePath);
                if (statsPath is not null) RejectSamePath(statsPath, tracePath);
            }
            using var traceWriter = tracePath is null ? null : new StreamWriter(new FileStream(tracePath, FileMode.CreateNew, FileAccess.Write));
            var blockTrace = traceWriter is null ? null : new BlockTrace(traceWriter, traceMb[0], traceMb[1], traceFrames);
            using var statistics = statsPath is null ? null : new StreamWriter(new FileStream(statsPath, FileMode.CreateNew, FileAccess.Write));
            var dcDiagnostics = statsPath is null ? null : new DcAverageDiagnostics(start + count - 1);
            using Stream? raw = outputPath is null ? null : binaryStdout ? Console.OpenStandardOutput()
                : new FileStream(outputPath, FileMode.CreateNew, FileAccess.Write);
            using var hashes = hashPath is null ? null : new StreamWriter(new FileStream(hashPath, FileMode.CreateNew, FileAccess.Write));
            var allocatedBefore = GC.GetTotalAllocatedBytes(precise: true);
            int[] collectionsBefore = [GC.CollectionCount(0), GC.CollectionCount(1), GC.CollectionCount(2)];
            var stopwatch = Stopwatch.StartNew();
            var decoder = new Bink2Decoder(container.Width, container.Height, dering, deringFrame, dcDiagnostics, exactChromaDcAverage, blockTrace);
            var written = 0;
            for (var index = 0; index < start + count; index++)
            {
                var packet = container.ReadNextFrame() ?? throw new InvalidDataException("Unexpected end of frame index.");
                var frame = decoder.Decode(packet.Data, packet.IsKeyFrame);
                if (index >= start)
                {
                    WriteFrame(frame, raw, hashes, index);
                    written++;
                }
                if ((index + 1) % 30 == 0) Console.Error.WriteLine($"Decoded {index + 1} frames; wrote {written}.");
            }
            raw?.Flush();
            hashes?.Flush();
            traceWriter?.Flush();
            stopwatch.Stop();
            var allocatedBytes = GC.GetTotalAllocatedBytes(precise: true) - allocatedBefore;
            int[] collections = [GC.CollectionCount(0) - collectionsBefore[0],
                GC.CollectionCount(1) - collectionsBefore[1], GC.CollectionCount(2) - collectionsBefore[2]];
            if (statistics is not null)
            {
                statistics.WriteLine(JsonSerializer.Serialize(dcDiagnostics!.Snapshot()));
                statistics.Flush();
            }
            using var process = Process.GetCurrentProcess();
#if DEBUG
            const string configuration = "Debug";
#else
            const string configuration = "Release";
#endif
            var summaryOutput = binaryStdout ? Console.Error : Console.Out;
            summaryOutput.WriteLine(JsonSerializer.Serialize(new { decodedFrames = start + count, writtenFrames = written,
                format = "yuv420p", range = "full", planeOrder = "Y,U,V", container.Width, container.Height,
                container.FrameRateNumerator, container.FrameRateDenominator, configuration,
                dering = deringName, deringFrame, dcAverageStats = statsPath, chromaDcAverage = chromaDcAverageName, blockTrace = tracePath, traceMb, traceFrames,
                elapsedMilliseconds = stopwatch.Elapsed.TotalMilliseconds,
                decodedFramesPerSecond = (start + count) / stopwatch.Elapsed.TotalSeconds,
                managedAllocatedBytes = allocatedBytes, gcCollections = collections,
                processPeakWorkingSetBytes = process.PeakWorkingSet64 }));
            return 0;
        }
        catch (Exception error) when (error is ArgumentException or IOException or NotSupportedException or OverflowException or FormatException or CryptographicException)
        {
            Console.Error.WriteLine($"{error.GetType().Name}: {error.Message}");
            return 1;
        }
    }

    private static Dictionary<string, string> ParseOptions(string[] args)
    {
        var options = new Dictionary<string, string>(StringComparer.Ordinal);
        for (var i = 0; i < args.Length; i += 2)
        {
            if (i + 1 == args.Length || args[i] is not ("--frames" or "--start-frame" or "--output" or "--hash-output" or "--dering" or "--dering-frame" or "--dc-average-stats" or "--chroma-dc-average" or "--block-trace" or "--trace-mb" or "--trace-frames") ||
                !options.TryAdd(args[i], args[i + 1]))
                throw new ArgumentException("Unknown, duplicate or incomplete decode option.");
        }
        return options;
    }

    private static int ParseNumber(string value) => int.Parse(value, NumberStyles.None, CultureInfo.InvariantCulture);

    private static void RejectSamePath(string first, string second)
    {
        if (string.Equals(Path.GetFullPath(first), Path.GetFullPath(second), StringComparison.OrdinalIgnoreCase))
            throw new ArgumentException("Input and output paths must be distinct.");
    }

    private static void WriteFrame(BinkFrame frame, Stream? raw, TextWriter? manifest, int index)
    {
        using var all = IncrementalHash.CreateHash(HashAlgorithmName.SHA256);
        var planeHashes = new string[3];
        for (var plane = 0; plane < 3; plane++)
        {
            using var hash = IncrementalHash.CreateHash(HashAlgorithmName.SHA256);
            var width = plane == 0 ? frame.Width : frame.Width / 2;
            var height = plane == 0 ? frame.Height : frame.Height / 2;
            for (var row = 0; row < height; row++)
            {
                var bytes = frame.Planes[plane].AsSpan(row * frame.Strides[plane], width);
                raw?.Write(bytes);
                hash.AppendData(bytes);
                all.AppendData(bytes);
            }
            planeHashes[plane] = Convert.ToHexString(hash.GetHashAndReset()).ToLowerInvariant();
        }
        manifest?.WriteLine(JsonSerializer.Serialize(new { frame = index, Y = planeHashes[0], U = planeHashes[1],
            V = planeHashes[2], YUV = Convert.ToHexString(all.GetHashAndReset()).ToLowerInvariant() }));
    }
}
