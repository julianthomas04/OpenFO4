using System.Buffers.Binary;

namespace OpenFallout4.Bink2;

internal static class SelfTests
{
    public static int Run()
    {
        var tests = new (string Name, Action Body)[]
        {
            ("LSB reads cross byte boundaries", CrossByteReads),
            ("32-bit reads retain the high bit", FullWidthRead),
            ("unaligned full-width reads preserve adjacent bits", UnalignedFullWidthRead),
            ("failed skips preserve bit position", SkipBounds),
            ("EOF rejection preserves reader position", EndOfInput),
            ("invalid bit counts preserve reader position", InvalidCounts),
            ("byte alignment discards only remaining byte bits", ByteAlignment),
            ("unary terminators are consumed", UnaryTerminators),
            ("unary cap does not consume the next bit", UnaryCap),
            ("invalid unary arguments preserve reader position", InvalidUnaryArguments),
            ("frame index preserves flags and packet boundaries", ValidContainer),
            ("logical size excludes trailing physical bytes", LogicalContainerEnd),
            ("invalid frame index boundaries are rejected", InvalidContainerIndex),
            ("truncated containers are rejected", TruncatedContainer),
            ("unsupported container features are rejected", UnsupportedContainer),
            ("audio track table is parsed and audio packets are skipped before the video packet", AudioContainer),
            ("DC reconstruction neighbors are near by absolute state distance", ReconstructionNeighborDistance),
            ("invalid audio tables and audio packets are rejected", InvalidAudioContainer),
            ("info states the container properties and audio tracks as JSON", InfoJson),
            ("only verified frame flag profiles are decoded", VerifiedFrameFlags),
            ("dering masks alter only selected output planes", DeringPlaneMasks),
            ("single-frame dering preserves baseline outside its source index", DeringFrameSelection),
            ("disabled dering preserves reconstruction and source filter flags", DeringReconstructionAndFlags),
            ("inter-coded edges inside macroblocks are filtered on the residual", InterEdgesFilterResidual),
            ("motion-forced internal edges filter the residual before the pixels", MotionForcedEdgesFilterResidualFirst),
            ("internal residual edges are filtered during macroblock decode, not in postprocess", ResidualEdgesFilteredAtDecode),
            ("residual edges between DC-only inter blocks are filtered only across large DC steps", DcOnlyResidualEdgeStrengths),
            ("internal residual edges on or below the visible bottom row are still filtered", ResidualEdgesBelowVisibleHeight),
            ("DC reconstruction of the bottom block row treats missing lower neighbors as absent", BottomRowReconstructionIgnoresMissingNeighbors),
            ("residual edges are filtered on the unclipped residual and added to the prediction", ResidualEdgesKeepUnclippedResidual),
            ("DC reconstruction covers coded rows below the visible height", ReconstructionCoversRowsBelowVisibleHeight),
            ("coding edges on the first row below the visible height are filtered", CodingEdgesBelowVisibleHeight),
            ("DC refresh diagnostics retain exact scale and rounding ties", DcAverageScale),
            ("DC refresh diagnostics isolate planes, frames and stride padding", DcAverageIsolation),
            ("exact chroma refresh changes only chroma DC predictors", ExactChromaRefreshIsolation),
            ("block tracing bounds frame selection and snapshots visible planes", BlockTraceSnapshots),
            ("block tracing retains the consumed neighbor DC slot", BlockTraceRefreshDependency),
            ("block tracing leaves postprocessed pixels unchanged", BlockTracePixelIsolation),
            ("block history is scalar while detailed snapshots remain frame gated", BlockTraceHistoryBounds),
            ("IDCT preserves DC sum before final quantization", TransformDcSum),
            ("IDCT DC scaling is exact", TransformDcScaling),
            ("intra IDCT clips into the destination while inter IDCT keeps the unclipped residual", TransformClippingContract),
            ("IDCT AC basis preserves axis and reflection parity", TransformAcAxes),
        };

        var failures = 0;
        foreach (var test in tests)
        {
            try
            {
                test.Body();
                Console.WriteLine($"PASS {test.Name}");
            }
            catch (Exception error)
            {
                failures++;
                Console.Error.WriteLine($"FAIL {test.Name}: {error}");
            }
        }
        Console.WriteLine($"Self-tests: {tests.Length - failures}/{tests.Length} passed");
        return failures == 0 ? 0 : 1;
    }

    private static void CrossByteReads()
    {
        var reader = new BinkBitReader([0x96, 0x35, 0xE1, 0x7A, 0xC3]);
        Equal(6u, reader.ReadBits(3));
        Equal(18u, reader.ReadBits(5));
        Equal(309u, reader.ReadBits(9));
        Equal(112u, reader.ReadBits(7));
        Equal(0xC37Au, reader.ReadBits(16));
        Equal(40, reader.Position);
        Equal(0, reader.BitsLeft);
    }

    private static void FullWidthRead()
    {
        var reader = new BinkBitReader([0x78, 0x56, 0x34, 0x92]);
        Equal(0x92345678u, reader.ReadBits(32));
        Equal(0u, reader.ReadBits(0));
        Equal(32, reader.Position);
    }

    private static void EndOfInput()
    {
        var empty = new BinkBitReader([]);
        Equal(0u, empty.ReadBits(0));
        Throws<InvalidDataException>(() => empty.ReadBits(1));
        Equal(0, empty.Position);

        var reader = new BinkBitReader([0x81]);
        Equal(1, reader.ReadBit());
        Throws<InvalidDataException>(() => reader.ReadBits(8));
        Equal(1, reader.Position);
        Equal(7, reader.BitsLeft);
        Equal(64u, reader.ReadBits(7));
        Throws<InvalidDataException>(() => reader.ReadBit());
        Equal(8, reader.Position);
    }

    private static void UnalignedFullWidthRead()
    {
        var reader = new BinkBitReader([0xDD, 0x9B, 0x57, 0x13, 0xF1]);
        Equal(5u, reader.ReadBits(3));
        Equal(0x226AF37Bu, reader.ReadBits(32));
        Equal(30u, reader.ReadBits(5));
        Equal(0, reader.BitsLeft);
    }

    private static void SkipBounds()
    {
        var reader = new BinkBitReader([0xAB, 0xCD]);
        reader.Skip(3);
        Throws<ArgumentOutOfRangeException>(() => reader.Skip(-1));
        Throws<InvalidDataException>(() => reader.Skip(14));
        Equal(3, reader.Position);
        reader.Skip(5);
        Equal(0xCDu, reader.ReadBits(8));
        reader.Skip(0);
        Throws<InvalidDataException>(() => reader.Skip(1));
        Equal(16, reader.Position);
    }

    private static void InvalidCounts()
    {
        var reader = new BinkBitReader([0xFF]);
        Throws<ArgumentOutOfRangeException>(() => reader.ReadBits(-1));
        Throws<ArgumentOutOfRangeException>(() => reader.ReadBits(33));
        Equal(0, reader.Position);
        Equal(255u, reader.ReadBits(8));
    }

    private static void ByteAlignment()
    {
        var reader = new BinkBitReader([0x96, 0x35]);
        Equal(6u, reader.ReadBits(3));
        reader.AlignByte();
        Equal(8, reader.Position);
        reader.AlignByte();
        Equal(8, reader.Position);
        Equal(0x35u, reader.ReadBits(8));
        reader.AlignByte();
        Equal(16, reader.Position);
    }

    private static void UnaryTerminators()
    {
        var stopOne = new BinkBitReader([0x28]);
        Equal(3, stopOne.ReadUnary(1, 7));
        Equal(4, stopOne.Position);
        Equal(0, stopOne.ReadBit());
        Equal(1, stopOne.ReadBit());

        var stopZero = new BinkBitReader([0xF7]);
        Equal(3, stopZero.ReadUnary(0, 7));
        Equal(4, stopZero.Position);
        Equal(1, stopZero.ReadBit());
        Throws<InvalidDataException>(() => new BinkBitReader([0]).ReadUnary(1, 9));
    }

    private static void UnaryCap()
    {
        var reader = new BinkBitReader([0x08]);
        Equal(0, reader.ReadUnary(1, 0));
        Equal(0, reader.Position);
        Equal(3, reader.ReadUnary(1, 3));
        Equal(3, reader.Position);
        Equal(1, reader.ReadBit());
        Equal(0, new BinkBitReader([]).ReadUnary(1, 0));
    }

    private static void InvalidUnaryArguments()
    {
        var reader = new BinkBitReader([0xFF]);
        Throws<ArgumentOutOfRangeException>(() => reader.ReadUnary(2, 3));
        Throws<ArgumentOutOfRangeException>(() => reader.ReadUnary(-1, 3));
        Throws<ArgumentOutOfRangeException>(() => reader.ReadUnary(1, -1));
        Equal(0, reader.Position);
    }

    private static byte[] SyntheticContainer()
    {
        var bytes = new byte[68];
        "KB2g"u8.CopyTo(bytes);
        PutField(bytes, 4, 60);
        PutField(bytes, 8, 2);
        PutField(bytes, 12, 8);
        PutField(bytes, 20, 64);
        PutField(bytes, 24, 2);
        PutField(bytes, 28, 30);
        PutField(bytes, 32, 1);
        PutField(bytes, 36, 0x10);
        PutField(bytes, 44, 52);
        PutField(bytes, 48, 61);
        for (var i = 0; i < 16; i++) bytes[52 + i] = (byte)i;
        return bytes;
    }

    private static byte[] SyntheticAudioContainer()
    {
        // Two tracks: 44-byte header, 24-byte audio table, two index entries, then the frames at 76 and 98.
        // Frame 0 carries a 6-byte packet for track 0 and an empty one for track 1; frame 1 has no audio.
        var bytes = new byte[114];
        "KB2g"u8.CopyTo(bytes);
        PutField(bytes, 4, 106);
        PutField(bytes, 8, 2);
        PutField(bytes, 12, 22);
        PutField(bytes, 20, 64);
        PutField(bytes, 24, 2);
        PutField(bytes, 28, 24000);
        PutField(bytes, 32, 1001);
        PutField(bytes, 36, 0x10);
        PutField(bytes, 40, 2);
        PutField(bytes, 44, 145920);
        PutField(bytes, 48, 4096);
        PutField(bytes, 52, 48000 | 0x7000u << 16);
        PutField(bytes, 56, 22050 | 0x1000u << 16);
        PutField(bytes, 60, 1);
        PutField(bytes, 64, 7);
        PutField(bytes, 68, 76);
        PutField(bytes, 72, 98);
        PutField(bytes, 76, 6);
        "AUDIO1"u8.CopyTo(bytes.AsSpan(80));
        PutField(bytes, 86, 0);
        for (var i = 0; i < 8; i++) bytes[90 + i] = (byte)(100 + i);
        PutField(bytes, 98, 0);
        PutField(bytes, 102, 0);
        for (var i = 0; i < 8; i++) bytes[106 + i] = (byte)(200 + i);
        return bytes;
    }

    private static void ReconstructionNeighborDistance()
    {
        const int threshold = 0x18 << 3;
        Equal(true, Bink2Decoder.ReconstructionNear(0xE000, 0xE00F, threshold));
        Equal(true, Bink2Decoder.ReconstructionNear(0xE00F, 0xE000, threshold));
        Equal(false, Bink2Decoder.ReconstructionNear(0xE000, 0xE000 + threshold, threshold));
        // A coded inter neighbor 0x8000 away must not count as near, and the distance does not wrap around 16 bits.
        Equal(false, Bink2Decoder.ReconstructionNear(0xE000, 0x6000, threshold));
        Equal(false, Bink2Decoder.ReconstructionNear(0xFFFF, 0x0000, threshold));
    }

    private static void AudioContainer()
    {
        WithContainer(SyntheticAudioContainer(), container =>
        {
            Equal(2, container.AudioTracks.Count);
            Equal(new Bink2AudioTrack(1, 48000, 0x7000, 145920), container.AudioTracks[0]);
            Equal(new Bink2AudioTrack(7, 22050, 0x1000, 4096), container.AudioTracks[1]);
            Equal(true, container.ReadNextFrame()!.Data.SequenceEqual(new byte[] { 100, 101, 102, 103, 104, 105, 106, 107 }));
            Equal(true, container.ReadNextFrame()!.Data.SequenceEqual(new byte[] { 200, 201, 202, 203, 204, 205, 206, 207 }));
        });
        WithContainer(SyntheticContainer(), container => Equal(0, container.AudioTracks.Count));
    }

    private static void VerifiedFrameFlags()
    {
        static Exception? DecodeWithFlags(uint flags)
        {
            var packet = new byte[256];
            BinaryPrimitives.WriteUInt32LittleEndian(packet, flags);
            try { new Bink2Decoder(64, 64).Decode(packet, keyFrame: true); return null; }
            catch (Exception error) { return error; }
        }
        foreach (var flags in new uint[] { 0xE000, 0x2018, 0x20FF, 0xE018 })
            Equal(false, DecodeWithFlags(flags) is NotSupportedException);
        foreach (var flags in new uint[] { 0xE019, 0x2000, 0x40000 | 0x2018 })
            Equal(true, DecodeWithFlags(flags) is NotSupportedException);
    }

    private static void InfoJson()
    {
        WithContainer(SyntheticAudioContainer(), container => Equal(
            "{\"width\":64,\"height\":2,\"frameCount\":2,\"frameRateNumerator\":24000,\"frameRateDenominator\":1001," +
            "\"audioTracks\":[{\"id\":1,\"sampleRate\":48000,\"channels\":2},{\"id\":7,\"sampleRate\":22050,\"channels\":1}]}",
            Program.InfoJson(container)));
        WithContainer(SyntheticContainer(), container => Equal(true, Program.InfoJson(container).EndsWith("\"audioTracks\":[]}", StringComparison.Ordinal)));
    }

    private static void InvalidAudioContainer()
    {
        var oversized = SyntheticAudioContainer();
        PutField(oversized, 86, 9);
        WithContainer(oversized, container => Throws<InvalidDataException>(() => container.ReadNextFrame()));
        var truncatedSize = SyntheticAudioContainer();
        PutField(truncatedSize, 76, 16);
        WithContainer(truncatedSize, container => Throws<InvalidDataException>(() => container.ReadNextFrame()));
        foreach (var (offset, value) in new (int, uint)[] { (40, 257), (52, 0x7000u << 16) })
        {
            var bytes = SyntheticAudioContainer();
            PutField(bytes, offset, value);
            Throws<InvalidDataException>(() => WithContainer(bytes, _ => { }));
        }
        foreach (var unknownFlag in new uint[] { 0x8000, 0x0001 })
        {
            var bytes = SyntheticAudioContainer();
            PutField(bytes, 52, 48000 | unknownFlag << 16);
            Throws<NotSupportedException>(() => WithContainer(bytes, _ => { }));
        }
    }

    private static void PutField(byte[] bytes, int offset, uint value) =>
        BinaryPrimitives.WriteUInt32LittleEndian(bytes.AsSpan(offset, 4), value);

    private static void WithContainer(byte[] bytes, Action<Bink2Container> check)
    {
        var path = Path.GetTempFileName();
        try
        {
            File.WriteAllBytes(path, bytes);
            using var container = Bink2Container.Open(path);
            check(container);
        }
        finally { File.Delete(path); }
    }

    private static void ValidContainer()
    {
        WithContainer(SyntheticContainer(), container =>
        {
            Equal(64, container.Width);
            Equal(2, container.Height);
            Equal(2, container.FrameCount);
            Equal(30u, container.FrameRateNumerator);
            Equal(1u, container.FrameRateDenominator);
            var first = container.ReadNextFrame() ?? throw new InvalidOperationException("Missing first frame.");
            Equal(0, first.Index);
            Equal(true, first.IsKeyFrame);
            Equal(8, first.Data.Length);
            Equal(true, first.Data.SequenceEqual(new byte[] { 0, 1, 2, 3, 4, 5, 6, 7 }));
            var second = container.ReadNextFrame() ?? throw new InvalidOperationException("Missing second frame.");
            Equal(1, second.Index);
            Equal(true, second.IsKeyFrame);
            Equal(true, second.Data.SequenceEqual(new byte[] { 8, 9, 10, 11, 12, 13, 14, 15 }));
            Equal(true, container.ReadNextFrame() is null);
            Equal(true, container.ReadNextFrame() is null);
        });
        var predicted = SyntheticContainer();
        PutField(predicted, 48, 60);
        WithContainer(predicted, container =>
        {
            _ = container.ReadNextFrame();
            Equal(false, container.ReadNextFrame()!.IsKeyFrame);
        });
    }

    private static void LogicalContainerEnd()
    {
        var bytes = SyntheticContainer().Concat(new byte[] { 255, 254, 253, 252 }).ToArray();
        WithContainer(bytes, container =>
        {
            _ = container.ReadNextFrame();
            var last = container.ReadNextFrame()!;
            Equal(8, last.Data.Length);
            Equal((byte)15, last.Data[^1]);
        });
    }

    private static void InvalidContainerIndex()
    {
        foreach (var (offset, value) in new (int, uint)[]
        {
            (44, 48), (48, 52), (48, 50), (48, 100), (12, 0), (12, 7), (8, 0), (8, uint.MaxValue),
        })
        {
            var bytes = SyntheticContainer();
            PutField(bytes, offset, value);
            Throws<InvalidDataException>(() => WithContainer(bytes, _ => { }));
        }
    }

    private static void TruncatedContainer()
    {
        var bytes = SyntheticContainer();
        foreach (var size in new[] { 0, 3, 43, 47, 51, 67 })
            Throws<InvalidDataException>(() => WithContainer(bytes[..size], _ => { }));
        PutField(bytes, 4, uint.MaxValue);
        Throws<InvalidDataException>(() => WithContainer(bytes, _ => { }));
        bytes = SyntheticContainer();
        PutField(bytes, 32, 0);
        Throws<InvalidDataException>(() => WithContainer(bytes, _ => { }));
    }

    private static void UnsupportedContainer()
    {
        foreach (var (offset, value) in new (int, uint)[] { (20, 32), (24, 3), (36, 0), (36, 0x11) })
        {
            var bytes = SyntheticContainer();
            PutField(bytes, offset, value);
            Throws<NotSupportedException>(() => WithContainer(bytes, _ => { }));
        }
        var signature = SyntheticContainer();
        signature[3] = (byte)'f';
        Throws<NotSupportedException>(() => WithContainer(signature, _ => { }));
    }

    private const ushort InterStrengthTwo = 0x4000;

    private sealed record PostprocessOptions
    {
        public int? SelectedFrame { get; init; }
        public int SourceFrame { get; init; }
        public uint Flags { get; init; } = 0x2018;
        public ushort State { get; init; } = 0xC000;
        public bool DefaultConstructor { get; init; }
        public BlockTrace? Trace { get; init; }
        public bool ResidualFromPixels { get; init; }
        public int[]? QuadrantOneMotion { get; init; }
        public bool DecodeStageResidualEdges { get; init; } = true;
        public bool Postprocess { get; init; } = true;
        public int Height { get; init; } = 64;
        public Func<int, int, int, int, ushort?>? StateOverride { get; init; }
        public byte? FlatPrediction { get; init; }
    }

    private static BinkFrame SyntheticPostprocess(DeringComponents mask, PostprocessOptions? options = null)
    {
        options ??= new PostprocessOptions();
        var decoder = options.DefaultConstructor
            ? new Bink2Decoder(64, options.Height)
            : new Bink2Decoder(64, options.Height, mask, options.SelectedFrame, blockTrace: options.Trace);
        decoder.frameFlags = options.Flags;
        decoder.frameIndex = options.SourceFrame;
        FillCheckerboard(decoder.output);
        if (options.ResidualFromPixels)
            SeedResidualFromPixels(decoder, options.FlatPrediction);
        AssignStates(decoder, options);
        if (options.DecodeStageResidualEdges)
            FilterDecodeStageResidualEdges(decoder, options.FlatPrediction);
        if (options.Postprocess) decoder.PostprocessFrame();
        return decoder.output;
    }

    private static void FillCheckerboard(BinkFrame frame)
    {
        for (var plane = 0; plane < 3; plane++)
        {
            var stride = frame.Strides[plane];
            for (var i = 0; i < frame.Planes[plane].Length; i++)
                frame.Planes[plane][i] = (byte)((i % stride / 8 + i / stride / 8) % 2 * 160);
        }
    }

    private static void SeedResidualFromPixels(Bink2Decoder decoder, byte? flatPrediction)
    {
        for (var plane = 0; plane < 3; plane++)
            for (var i = 0; i < decoder.residualPlanes[plane].Length; i++)
            {
                decoder.residualPlanes[plane][i] = decoder.output.Planes[plane][i];
                if (flatPrediction.HasValue)
                    decoder.output.Planes[plane][i] = (byte)Math.Min(255, flatPrediction.Value + decoder.residualPlanes[plane][i]);
            }
    }

    private static void AssignStates(Bink2Decoder decoder, PostprocessOptions options)
    {
        var rows = decoder.frameState;
        for (var mbY = 0; mbY < rows.Length; mbY++)
            for (var mbX = 0; mbX < rows[mbY].Length; mbX++)
            {
                var block = rows[mbY][mbX];
                for (var component = 0; component < block.State.Length; component++)
                    for (var index = 0; index < block.State[component].Length; index++)
                        block.State[component][index] = options.StateOverride?.Invoke(mbX, mbY, component, index) ?? options.State;
                options.QuadrantOneMotion?.CopyTo(block.Motion[1], 0);
            }
    }

    private static void FilterDecodeStageResidualEdges(Bink2Decoder decoder, byte? flatPrediction)
    {
        var frame = decoder.output;
        var rows = decoder.frameState;
        for (var mbY = 0; mbY < rows.Length; mbY++)
            for (var mbX = 0; mbX < rows[mbY].Length; mbX++)
            {
                for (var plane = 0; plane < 3; plane++)
                {
                    var side = plane == 0 ? 32 : 16;
                    var stride = frame.Strides[plane];
                    var origin = mbY * side * stride + mbX * side;
                    for (var i = 0; i < side * side; i++)
                    {
                        var position = origin + i / side * stride + i % side;
                        decoder.prediction[plane][i] = flatPrediction ?? (byte)(frame.Planes[plane][position] - decoder.residualPlanes[plane][position]);
                    }
                }
                decoder.AddFilteredResidual(rows[mbY][mbX], mbX * 32, mbY * 32);
            }
    }

    private static void DeringPlaneMasks()
    {
        var none = SyntheticPostprocess(DeringComponents.None);
        var all = SyntheticPostprocess(DeringComponents.All);
        var defaults = SyntheticPostprocess(DeringComponents.None, new() { DefaultConstructor = true });
        var luma = SyntheticPostprocess(DeringComponents.Luma);
        var chroma = SyntheticPostprocess(DeringComponents.Chroma);
        for (var plane = 0; plane < 3; plane++)
        {
            Equal(false, all.Planes[plane].SequenceEqual(none.Planes[plane]));
            Equal(true, defaults.Planes[plane].SequenceEqual(all.Planes[plane]));
            Equal(true, luma.Planes[plane].SequenceEqual(plane == 0 ? all.Planes[plane] : none.Planes[plane]));
            Equal(true, chroma.Planes[plane].SequenceEqual(plane == 0 ? none.Planes[plane] : all.Planes[plane]));
        }
    }

    private static void DeringFrameSelection()
    {
        var baseline = SyntheticPostprocess(DeringComponents.All);
        var disabled = SyntheticPostprocess(DeringComponents.None);
        foreach (var sourceIndex in new[] { 4, 5, 6 })
        {
            var actual = SyntheticPostprocess(DeringComponents.None, new() { SelectedFrame = 5, SourceFrame = sourceIndex });
            var expected = sourceIndex == 5 ? disabled : baseline;
            for (var plane = 0; plane < 3; plane++) Equal(true, actual.Planes[plane].SequenceEqual(expected.Planes[plane]));
        }
    }

    private static void DeringReconstructionAndFlags()
    {
        var reconstructed = SyntheticPostprocess(DeringComponents.None, new() { State = 0xE400 });
        for (var plane = 0; plane < 3; plane++)
            Equal(true, reconstructed.Planes[plane].All(pixel => pixel == 128));
        var blocked = SyntheticPostprocess(DeringComponents.All, new() { Flags = 0xE000 });
        var disabled = SyntheticPostprocess(DeringComponents.None);
        for (var plane = 0; plane < 3; plane++) Equal(true, blocked.Planes[plane].SequenceEqual(disabled.Planes[plane]));
    }

    private static void InterEdgesFilterResidual()
    {
        var unfiltered = SyntheticPostprocess(DeringComponents.None, new() { State = InterStrengthTwo });
        var flatResidual = SyntheticPostprocess(DeringComponents.All, new() { State = InterStrengthTwo });
        var steppedResidual = SyntheticPostprocess(DeringComponents.All, new() { State = InterStrengthTwo, ResidualFromPixels = true });
        for (var plane = 0; plane < 3; plane++)
        {
            Equal(true, flatResidual.Planes[plane].SequenceEqual(unfiltered.Planes[plane]));
            var stride = unfiltered.Strides[plane];
            var macroblockSide = plane == 0 ? 32 : 16;
            // Rows 2..5 of each block are outside the reach of horizontal-edge taps.
            for (var y = 0; y < macroblockSide; y++)
            {
                if (y % 8 is < 2 or > 5) continue;
                var row = y * stride;
                Equal(false, steppedResidual.Planes[plane][row + 8] == unfiltered.Planes[plane][row + 8]);
                Equal(true, steppedResidual.Planes[plane][row + macroblockSide] == unfiltered.Planes[plane][row + macroblockSide]);
                Equal(true, steppedResidual.Planes[plane][row + macroblockSide - 1] == unfiltered.Planes[plane][row + macroblockSide - 1]);
            }
        }
    }

    private static void MotionForcedEdgesFilterResidualFirst()
    {
        int[] forcingMotion = [4, 0];
        var forced = SyntheticPostprocess(DeringComponents.All, new() { State = InterStrengthTwo, ResidualFromPixels = true, QuadrantOneMotion = forcingMotion });
        // Row 3 lies outside the reach of horizontal-edge taps. Across the luma quadrant edge at x = 16 the
        // pixels and the residual both step from 160 to 0. The residual pass moves the four taps to
        // 140/120/40/20, then the forced pixel pass on that 80 step yields 130/100/60/30.
        var row = 3 * forced.Strides[0];
        Equal(true, forced.Planes[0].AsSpan(row + 14, 4).SequenceEqual(new byte[] { 130, 100, 60, 30 }));
    }

    private static void ResidualEdgesFilteredAtDecode()
    {
        var decoded = SyntheticPostprocess(DeringComponents.All, new() { State = InterStrengthTwo, ResidualFromPixels = true, Postprocess = false });
        var postprocessedOnly = SyntheticPostprocess(DeringComponents.All, new() { State = InterStrengthTwo, ResidualFromPixels = true, DecodeStageResidualEdges = false });
        // Row 3 of the first block row, across the internal luma edge at x = 8 where the residual steps from 0 to 160.
        var row = 3 * decoded.Strides[0];
        Equal(true, decoded.Planes[0].AsSpan(row + 6, 4).SequenceEqual(new byte[] { 20, 40, 120, 140 }));
        Equal(true, postprocessedOnly.Planes[0].AsSpan(row + 6, 4).SequenceEqual(new byte[] { 0, 0, 160, 160 }));
    }

    private static void ReconstructionCoversRowsBelowVisibleHeight()
    {
        const ushort dcOnlyIntra = 0xE000 | 800;
        // A 56-row picture is coded as 64 rows; the coded rows 56..63 stay part of the reference picture.
        var frame = SyntheticPostprocess(DeringComponents.None, new() { State = dcOnlyIntra, Height = 56 });
        var stride = frame.Strides[0];
        for (var y = 56; y < 64; y++)
            for (var x = 0; x < 64; x++)
                Equal((byte)100, frame.Planes[0][y * stride + x]);
    }

    private static void CodingEdgesBelowVisibleHeight()
    {
        const ushort codedIntraStrengthTwo = 0xC000;
        // A 56-row picture is coded as 64 rows. Column 3 steps from 0 to 160 across the edge at y = 56 and is
        // out of reach of the vertical edges, so only that edge moves rows 54 and 55 by its outer and inner taps.
        var frame = SyntheticPostprocess(DeringComponents.All, new() { State = codedIntraStrengthTwo, Height = 56 });
        var stride = frame.Strides[0];
        Equal((byte)20, frame.Planes[0][54 * stride + 3]);
        Equal((byte)40, frame.Planes[0][55 * stride + 3]);
    }

    private static void ResidualEdgesKeepUnclippedResidual()
    {
        var frame = SyntheticPostprocess(DeringComponents.All, new() { State = InterStrengthTwo, ResidualFromPixels = true, Postprocess = false, FlatPrediction = 120 });
        // Row 3, across the internal luma edge at x = 8: prediction 120, residual stepping 0 -> 160. The
        // filtered residual 20/40/120/140 is added to the prediction, so 120 + 160 overshooting 255 must not
        // be clipped before the edge filter.
        var row = 3 * frame.Strides[0];
        Equal(true, frame.Planes[0].AsSpan(row + 6, 4).SequenceEqual(new byte[] { 140, 160, 240, 255 }));
    }

    private static void DcOnlyResidualEdgeStrengths()
    {
        (int, int) Strengths(int strength, int firstDc, int secondDc) =>
            Bink2Decoder.ResidualEdgeStrengths((ushort)(strength << 13 | (firstDc & 0x1FFF)), (ushort)(strength << 13 | (secondDc & 0x1FFF)));
        Equal((3, 3), Strengths(3, 16, -16));
        Equal((2, 2), Strengths(3, 1, 34));
        Equal((2, 2), Strengths(3, -20, 16));
        Equal((2, 2), Strengths(2, 0, 100));
        Equal((1, 1), Strengths(1, 0, 100));
    }

    private static void ResidualEdgesBelowVisibleHeight()
    {
        // A 56-row picture is coded as 64 rows; the internal edge at y = 56 lies on the first invisible row.
        var filtered = SyntheticPostprocess(DeringComponents.All, new() { State = InterStrengthTwo, ResidualFromPixels = true, Height = 56, Postprocess = false });
        var unfiltered = SyntheticPostprocess(DeringComponents.None, new() { State = InterStrengthTwo, ResidualFromPixels = true, Height = 56, Postprocess = false });
        var lastVisibleRow = 55 * filtered.Strides[0];
        Equal(false, filtered.Planes[0][lastVisibleRow + 3] == unfiltered.Planes[0][lastVisibleRow + 3]);
    }

    private static void BottomRowReconstructionIgnoresMissingNeighbors()
    {
        const ushort dcOnlyIntra = 0xE000 | 800;
        const ushort codedIntraNearDc = 0xC000 | 980;
        // Chroma plane of a 56-row picture: the last block row starts at row 24 and has no lower neighbors.
        // Block index 3 of the first macroblock is the target (x 8..16). A horizontal neighbor that is not DC-only
        // but carries a nearby DC must not leak into the target's lower corner on its side.
        foreach (var (mbX, index) in new[] { (0, 2), (1, 2) })
        {
            var frame = SyntheticPostprocess(DeringComponents.None, new()
            {
                Height = 56,
                StateOverride = (x, y, component, i) => component != 0 && x == mbX && y == 1 && i == index ? codedIntraNearDc : dcOnlyIntra,
            });
            var stride = frame.Strides[1];
            for (var y = 24; y < 28; y++)
                for (var x = 8; x < 16; x++)
                    Equal((byte)100, frame.Planes[1][y * stride + x]);
        }
    }

    private static void TransformClippingContract()
    {
        const int dc = 3000;
        const byte prediction = 100;
        short[] LargeDcBlock()
        {
            var block = new short[64];
            block[0] = (short)(8 * dc + 32);
            return block;
        }
        var unclippedStep = (short)((dc + 4) >> 3);

        var intra = new byte[80];
        Bink2Decoder.IdctIntra(intra, 10, LargeDcBlock());
        for (var y = 0; y < 8; y++)
        {
            for (var x = 0; x < 8; x++) Equal((byte)255, intra[y * 10 + x]);
            Equal((byte)0, intra[y * 10 + 8]);
        }

        var residual = new short[80];
        Bink2Decoder.IdctResidual(residual, 10, LargeDcBlock());
        var final = new byte[80];
        for (var y = 0; y < 8; y++)
            for (var x = 0; x < 8; x++)
            {
                Equal(unclippedStep, residual[y * 10 + x]);
                final[y * 10 + x] = (byte)Math.Clamp(prediction + residual[y * 10 + x], 0, 255);
                Equal((byte)255, final[y * 10 + x]);
            }
        Equal((short)0, residual[8]);
        Equal((short)0, residual[9]);

        // A lowered residual keeps its full range: -375 may not be clamped to -128 before the final clip.
        var negativeBlock = LargeDcBlock();
        negativeBlock[0] = (short)(-8 * dc + 32);
        Bink2Decoder.IdctResidual(residual, 10, negativeBlock);
        Equal((short)((-dc + 4) >> 3), residual[0]);
    }

    private static byte[] MaximumAverageBiasBlock()
    {
        byte[] rows = [0, 1, 1, 2, 1, 2, 2, 3];
        var block = new byte[64];
        for (var row = 0; row < 8; row++) Array.Fill(block, rows[row], row * 8, 8);
        return block;
    }

    private static void DcAverageScale()
    {
        var diagnostics = new DcAverageDiagnostics(5);
        diagnostics.Observe(5, 0, MaximumAverageBiasBlock(), 8, 24);
        var snapshot = diagnostics.Snapshot();
        var y = snapshot.Overall[0];
        Equal("Y", y.Plane);
        Equal(1L, y.Calls);
        Equal(24L, y.TreeDcSum);
        Equal(96L, y.ExactSum64);
        Equal(96L, y.ScaledBiasSum);
        Equal(12.0, y.MeanDcBias);
        Equal(1.5, y.MeanPixelEquivalentBias);
        Equal(96, y.MinimumScaledBias!.Value);
        Equal(96, y.MaximumScaledBias!.Value);
        Equal(1L, y.ScaledBiasHistogram[96]);
        Equal(1L, y.FirstLevelTieHistogram[32]);
        Equal(1L, y.SecondLevelTieHistogram[16]);
        Equal(1L, y.ThirdLevelTieHistogram[8]);
        Equal(1L, y.FirstLevelTieHistogram.Sum());
        Equal(1L, snapshot.FinalFrame[0].Calls);
    }

    private static void DcAverageIsolation()
    {
        var diagnostics = new DcAverageDiagnostics(5);
        var padded = new byte[80];
        Array.Fill(padded, (byte)255);
        for (var row = 0; row < 8; row++) Array.Fill(padded, (byte)128, row * 10, 8);
        diagnostics.Observe(0, 1, padded, 10, 1024);
        diagnostics.Observe(5, 1, MaximumAverageBiasBlock(), 8, 24);
        diagnostics.Observe(6, 2, padded, 10, 1024);
        var snapshot = diagnostics.Snapshot();
        Equal(5, snapshot.FinalSourceFrame);
        Equal(0L, snapshot.Overall[0].Calls);
        Equal("U", snapshot.Overall[1].Plane);
        Equal("V", snapshot.Overall[2].Plane);
        var u = snapshot.Overall[1];
        Equal(2L, u.Calls);
        Equal(1048L, u.TreeDcSum);
        Equal(8288L, u.ExactSum64);
        Equal(96L, u.ScaledBiasSum);
        Equal(6.0, u.MeanDcBias);
        Equal(0.75, u.MeanPixelEquivalentBias);
        Equal(0, u.MinimumScaledBias!.Value);
        Equal(96, u.MaximumScaledBias!.Value);
        Equal(1L, u.ScaledBiasHistogram[0]);
        Equal(1L, u.ScaledBiasHistogram[96]);
        Equal(1L, u.FirstLevelTieHistogram[0]);
        Equal(1L, snapshot.FinalFrame[1].Calls);
        Equal(1.5, snapshot.FinalFrame[1].MeanPixelEquivalentBias);
        Equal(0L, snapshot.FinalFrame[2].Calls);
        Equal(0L, snapshot.Overall[2].ScaledBiasSum);
    }

    private static void ExactChromaRefreshIsolation()
    {
        foreach (var mode in new[] { "default", "staged", "exact" })
        {
            var decoder = mode == "default" ? new Bink2Decoder(64, 64)
                : new Bink2Decoder(64, 64, DeringComponents.All, exactChromaDcAverage: mode == "exact");
            var frame = decoder.output;
            byte[] rowPattern = [0, 1, 1, 2, 1, 2, 2, 3];
            for (var plane = 0; plane < 3; plane++)
                for (var i = 0; i < frame.Planes[plane].Length; i++)
                    frame.Planes[plane][i] = rowPattern[i / frame.Strides[plane] % 8];
            var original = frame.Planes.Select(plane => (byte[])plane.Clone()).ToArray();
            var block = decoder.current[0];
            decoder.AverageMacroblock(block, 0, 0);
            var dc = block.Dc;
            for (var component = 0; component < 3; component++)
            {
                var expected = mode == "exact" && component != 0 ? 12 : 24;
                var count = component == 0 ? 16 : 4;
                for (var i = 0; i < count; i++) Equal(expected, dc[component][i]);
            }
            for (var plane = 0; plane < 3; plane++) Equal(true, frame.Planes[plane].SequenceEqual(original[plane]));
        }
    }

    private static void BlockTraceSnapshots()
    {
        using var writer = new StringWriter(System.Globalization.CultureInfo.InvariantCulture);
        int[] selected = [10, 20];
        var trace = new BlockTrace(writer, 1, 1, selected);
        selected[0] = 11;
        trace.ValidateDimensions(64, 64);
        Throws<ArgumentException>(() => new BlockTrace(writer, 2, 1, [10]).ValidateDimensions(64, 64));
        Throws<ArgumentException>(() => _ = new BlockTrace(writer, 0, 0, [1, 2, 3, 4]));
        Throws<ArgumentException>(() => _ = new BlockTrace(writer, 0, 0, [10, 10]));
        var frame = new BinkFrame(64, 64);
        for (var plane = 0; plane < 3; plane++)
            for (var i = 0; i < frame.Planes[plane].Length; i++)
                frame.Planes[plane][i] = (byte)(17 * plane + i / frame.Strides[plane] + i % frame.Strides[plane]);
        trace.Region(11, "unselected", frame);
        Equal("", writer.ToString());
        trace.Region(10, "before", frame);
        foreach (var plane in frame.Planes) Array.Clear(plane);
        trace.Region(20, "after", frame);
        var lines = writer.ToString().Split(Environment.NewLine, StringSplitOptions.RemoveEmptyEntries);
        Equal(2, lines.Length);
        using var before = System.Text.Json.JsonDocument.Parse(lines[0]);
        using var after = System.Text.Json.JsonDocument.Parse(lines[1]);
        Equal(10, before.RootElement.GetProperty("frame").GetInt32());
        var planes = before.RootElement.GetProperty("data").GetProperty("planes");
        for (var plane = 0; plane < 3; plane++)
        {
            var samples = planes[plane].GetProperty("samples");
            var side = plane == 0 ? 16 : 8;
            Equal(side, samples.GetProperty("width").GetInt32());
            Equal(side * side, samples.GetProperty("pixels").GetArrayLength());
            Equal(plane == 0 ? 64 : 32 + 17 * plane, samples.GetProperty("pixels")[0].GetInt32());
            Equal(0, after.RootElement.GetProperty("data").GetProperty("planes")[plane].GetProperty("samples").GetProperty("pixels")[0].GetInt32());
        }
    }

    private static void BlockTraceRefreshDependency()
    {
        using var writer = new StringWriter(System.Globalization.CultureInfo.InvariantCulture);
        var trace = new BlockTrace(writer, 3, 2, [10]);
        var pixels = MaximumAverageBiasBlock();
        trace.Refresh(10, 3, 2, 2, 2, 2, 1, 0, pixels, 8, 24, 24);
        trace.Refresh(9, 3, 2, 2, 2, 2, 1, 1, pixels, 8, 24, 24);
        Equal("", writer.ToString());
        trace.Refresh(10, 3, 2, 2, 2, 2, 1, 1, pixels, 8, 24, 24);
        Array.Clear(pixels);
        using var record = System.Text.Json.JsonDocument.Parse(writer.ToString());
        var data = record.RootElement.GetProperty("data");
        Equal(2, data.GetProperty("plane").GetInt32());
        Equal(1, data.GetProperty("index").GetInt32());
        Equal(96, data.GetProperty("sum64").GetInt32());
        Equal(24, data.GetProperty("usedDc").GetInt32());
        Equal(12, data.GetProperty("exactDc").GetInt32());
        Equal(1, data.GetProperty("pixels")[8].GetInt32());
    }

    private static void BlockTracePixelIsolation()
    {
        var baseline = SyntheticPostprocess(DeringComponents.All, new() { SourceFrame = 10 });
        using var writer = new StringWriter(System.Globalization.CultureInfo.InvariantCulture);
        var trace = new BlockTrace(writer, 0, 0, [10]);
        var observed = SyntheticPostprocess(DeringComponents.All, new() { SourceFrame = 10, Trace = trace });
        for (var plane = 0; plane < 3; plane++) Equal(true, observed.Planes[plane].SequenceEqual(baseline.Planes[plane]));
        var lines = writer.ToString().Split(Environment.NewLine, StringSplitOptions.RemoveEmptyEntries);
        Equal(1, lines.Length);
        using var record = System.Text.Json.JsonDocument.Parse(lines[0]);
        Equal("after-selected-row-reconstruction-before-row-filters", record.RootElement.GetProperty("stage").GetString()!);
    }

    private static void BlockTraceHistoryBounds()
    {
        using var writer = new StringWriter(System.Globalization.CultureInfo.InvariantCulture);
        var trace = new BlockTrace(writer, 0, 0, [4]);
        int[][] motion = [new int[2], new int[2], new int[2], new int[2]];
        int[][] dc = [new int[16], new int[16], new int[16]];
        ushort[][] state = [new ushort[16], new ushort[16], new ushort[16]];
        trace.Macroblock(3, "macroblock-begin", 0, 0, 2, 0, 10, 11, 12, motion, dc, state);
        trace.Macroblock(3, "macroblock-end", 1, 0, 2, 0, 10, 11, 12, motion, dc, state);
        Equal("", writer.ToString());
        trace.Macroblock(3, "macroblock-end", 0, 0, 2, 0, 100, 11, 12, motion, dc, state);
        trace.Region(3, "unselected", new BinkFrame(64, 64));
        trace.DcOutput(3, 0, 0, 0, dc[0]);
        trace.Macroblock(4, "macroblock-end", 0, 0, 3, 0, 200, 13, 14, motion, dc, state);
        var lines = writer.ToString().Split(Environment.NewLine, StringSplitOptions.RemoveEmptyEntries);
        Equal(3, lines.Length);
        for (var i = 0; i < 2; i++)
        {
            using var record = System.Text.Json.JsonDocument.Parse(lines[i]);
            Equal("macroblock-history", record.RootElement.GetProperty("stage").GetString()!);
            Equal(3 + i, record.RootElement.GetProperty("frame").GetInt32());
            var data = record.RootElement.GetProperty("data");
            Equal(i == 0 ? 100 : 200, data.GetProperty("bitPosition").GetInt32());
            Equal(5, data.EnumerateObject().Count());
            foreach (var field in data.EnumerateObject())
                Equal(true, field.Value.ValueKind == System.Text.Json.JsonValueKind.Number);
        }
        using var detailed = System.Text.Json.JsonDocument.Parse(lines[2]);
        Equal("macroblock-end", detailed.RootElement.GetProperty("stage").GetString()!);
        Equal(4, detailed.RootElement.GetProperty("frame").GetInt32());
        Equal(true, detailed.RootElement.GetProperty("data").GetProperty("motion").ValueKind == System.Text.Json.JsonValueKind.Array);
    }

    private static void TransformDcSum()
    {
        for (var sample = 0; sample < 64; sample++)
        {
            var vector = new short[8];
            for (var i = 0; i < vector.Length; i++)
                vector[i] = (short)((sample * 29 + i * 47) % 129 - 64);
            var expectedSum = 8 * vector[0];
            Bink2Decoder.Idct1D(vector, 1, 0);
            Equal(expectedSum, vector.Sum(value => (int)value));

            var block = new short[64];
            for (var i = 1; i < block.Length; i++)
                block[i] = (short)((sample * 17 + i * 31) % 49 - 24);
            var dc = sample % 17 - 8;
            block[0] = (short)(8 * dc + 32);
            var residual = new short[64];
            Bink2Decoder.IdctResidual(residual, 8, block);
            var sum = block.Sum(value => (int)value);
            // Pair cancellation gives sum(unshifted) = 64 * input DC.
            // Each final floor loses at most 63/64: sum - 8*dc is [-31, 32].
            Equal(true, sum - 8 * dc >= -31 && sum - 8 * dc <= 32);
            for (var y = 0; y < 8; y++)
                for (var x = 0; x < 8; x++)
                {
                    Equal(block[x * 8 + y], residual[y * 8 + x]);
                    Equal(true, Math.Abs((int)residual[y * 8 + x]) < 127);
                }
        }
    }

    private static void TransformDcScaling()
    {
        foreach (var dc in new[] { -900, -33, -12, -5, -4, -3, -1, 0, 1, 3, 4, 5, 11, 12, 33, 900 })
        {
            var block = new short[64];
            block[0] = (short)(8 * dc + 32);
            var residual = new short[80];
            Bink2Decoder.IdctResidual(residual, 10, block);
            var expected = (short)((dc + 4) >> 3);
            for (var y = 0; y < 8; y++)
            {
                for (var x = 0; x < 8; x++) Equal(expected, residual[y * 10 + x]);
                Equal((short)0, residual[y * 10 + 8]);
                Equal((short)0, residual[y * 10 + 9]);
            }
        }
    }

    private static void TransformAcAxes()
    {
        foreach (var frequency in new[] { 1, 2, 3, 4, 5, 6, 7 })
            foreach (var amplitude in new[] { -256, 256 })
                foreach (var vertical in new[] { false, true })
                {
                    var block = new short[64];
                    block[vertical ? frequency : frequency * 8] = (short)amplitude;
                    var residual = new short[64];
                    Bink2Decoder.IdctResidual(residual, 8, block);
                    var axis = new int[8];
                    for (var i = 0; i < 8; i++)
                    {
                        axis[i] = residual[vertical ? i * 8 : i];
                        for (var j = 0; j < 8; j++)
                            Equal(axis[i], (int)residual[vertical ? i * 8 + j : j * 8 + i]);
                    }
                    Equal(true, axis.Min() < 0 && axis.Max() > 0);
                    for (var i = 0; i < 4; i++)
                        if ((frequency & 1) == 0)
                            Equal(axis[i], axis[7 - i]);
                        else
                            Equal(true, axis[i] + axis[7 - i] is -1 or 0);
                }
    }

    private static void Equal<T>(T expected, T actual) where T : IEquatable<T>
    {
        if (!expected.Equals(actual))
            throw new InvalidOperationException($"Expected {expected}; got {actual}.");
    }

    private static void Throws<T>(Action action) where T : Exception
    {
        try
        {
            action();
        }
        catch (T)
        {
            return;
        }
        throw new InvalidOperationException($"Expected {typeof(T).Name}.");
    }
}
