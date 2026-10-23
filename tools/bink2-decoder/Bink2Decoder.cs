// SPDX-License-Identifier: GPL-3.0-or-later
// Derived from the Bink2 decoder by Konstantin Shishkov (2014) and Paul B Mahol (2019).
using System.Buffers.Binary;

namespace OpenFallout4.Bink2;

public sealed partial class Bink2Decoder
{
    internal sealed class Macroblock
    {
        internal readonly int[][] Dc = { new int[16], new int[16], new int[16] };
        internal readonly ushort[][] State = { new ushort[16], new ushort[16], new ushort[16] };
        internal readonly int[][] Motion = { new int[2], new int[2], new int[2], new int[2] };
        internal int Type, IntraQ, InterQ;
        internal void Clear()
        {
            foreach (int[] values in Dc) Array.Clear(values);
            foreach (ushort[] values in State) Array.Clear(values);
            foreach (int[] values in Motion) Array.Clear(values);
            Type = IntraQ = InterQ = 0;
        }
    }
    private readonly int width, height, macroblockWidth;
    internal Macroblock[] current;
    private Macroblock[] previous;
    internal readonly Macroblock[][] frameState;
    internal BinkFrame output;
    private BinkFrame reference;
    internal readonly short[][] residualPlanes;
    internal readonly byte[][] prediction;
    private BinkBitReader bits = null!;
    private static readonly uint[] VerifiedFrameFlags = [0xE000, 0x2018, 0x20FF, 0xE018];
    internal uint frameFlags;
    private int mbPosition, component;
    private bool haveReference;
    internal int frameIndex;
    private readonly DeringComponents deringComponents;
    private readonly int? deringFrame;
    private readonly DcAverageDiagnostics? dcAverageDiagnostics;
    private readonly bool exactChromaDcAverage;
    private readonly BlockTrace? blockTrace;
    private int macroblockY;

    public Bink2Decoder(int width, int height) : this(width, height, DeringComponents.All)
    {
    }
    internal Bink2Decoder(int width, int height, DeringComponents deringComponents, int? deringFrame = null, DcAverageDiagnostics? dcAverageDiagnostics = null, bool exactChromaDcAverage = false, BlockTrace? blockTrace = null)
    {
        if ((deringComponents & ~DeringComponents.All) != 0)
            throw new ArgumentOutOfRangeException(nameof(deringComponents));
        if (deringFrame < 0)
            throw new ArgumentOutOfRangeException(nameof(deringFrame));
        this.deringComponents = deringComponents;
        this.deringFrame = deringFrame;
        this.dcAverageDiagnostics = dcAverageDiagnostics;
        this.exactChromaDcAverage = exactChromaDcAverage;
        this.blockTrace = blockTrace;
        blockTrace?.ValidateDimensions(width, height);
        if (width < 64 || height <= 0 || (width & 31) != 0 || (height & 1) != 0 || width > 4096 || height > 4096)
            throw new NotSupportedException("Decoder requires even-height KB2g, width at least 64 and aligned to 32, up to 4096 pixels per dimension.");
        this.width = width; this.height = height;
        macroblockWidth = (width + 31) / 32;
        current = MakeRow(); previous = MakeRow();
        frameState = new Macroblock[(height + 31) / 32][];
        for (int row = 0; row < frameState.Length; row++) frameState[row] = MakeRow();
        output = new BinkFrame(width, height); reference = new BinkFrame(width, height);
        residualPlanes = output.Planes.Select(plane => new short[plane.Length]).ToArray();
        prediction = [new byte[32 * 32], new byte[16 * 16], new byte[16 * 16]];
    }
    private Macroblock[] MakeRow()
    {
        var row = new Macroblock[macroblockWidth];
        for (int i = 0; i < row.Length; i++) row[i] = new Macroblock();
        return row;
    }
    public BinkFrame Decode(byte[] packet, bool keyFrame)
    {
        ArgumentNullException.ThrowIfNull(packet);
        if (packet.Length < 8) throw new InvalidDataException("Truncated Bink2 frame header.");
        if (!keyFrame && !haveReference) throw new InvalidDataException("Predicted frame has no reference.");
        frameFlags = BinaryPrimitives.ReadUInt32LittleEndian(packet);
        if (Array.IndexOf(VerifiedFrameFlags, frameFlags) < 0) throw new NotSupportedException($"Frame flags 0x{frameFlags:X} are not among the verified KB2g profiles.");
        int codedHeight = (height + 31) & ~31;
        int slices = codedHeight < 128 ? 1 : 2;
        int firstEnd = slices == 1 ? codedHeight : (height / 2 + 16) & ~31;
        int firstPacketEnd = slices == 1 ? packet.Length : checked((int)BinaryPrimitives.ReadUInt32LittleEndian(packet.AsSpan(4)));
        if (firstPacketEnd < slices * 4 || firstPacketEnd > packet.Length) throw new InvalidDataException("Invalid slice offset.");
        foreach (short[] plane in residualPlanes) Array.Clear(plane);
        bits = new BinkBitReader(packet);
        bits.Skip(slices * 32);
        DecodeSlice(keyFrame, 0, firstEnd);
        bits.AlignByte();
        if (bits.Position > firstPacketEnd * 8) throw new InvalidDataException("Slice overread.");
        bits.Skip(firstPacketEnd * 8 - bits.Position);
        if (slices == 2)
        {
            DecodeSlice(keyFrame, firstEnd, codedHeight);
            bits.AlignByte();
            if (bits.Position > packet.Length * 8) throw new InvalidDataException("Slice overread.");
        }
        blockTrace?.Region(frameIndex, "before-frame-postprocess", output);
        if ((frameFlags & FrameFlagEdgeFilters) != 0) PostprocessFrame();
        blockTrace?.Region(frameIndex, "after-frame-postprocess", output);
        haveReference = true;
        frameIndex++;
        BinkFrame decoded = output;
        (output, reference) = (reference, output);
        return decoded;
    }
    private int UpdateQ(bool intra, int dq, int flags)
    {
        int Get(Macroblock b) => intra ? b.IntraQ : b.InterQ;
        int q = (flags & 0xa0) == 0xa0 ? 16 : (flags & 0x80) != 0 ? Get(current[mbPosition - 1]) : (flags & 0x20) != 0 ? Get(previous[mbPosition]) : Median(Get(previous[mbPosition]), Get(current[mbPosition - 1]), Get(previous[mbPosition - 1]));
        q = unchecked((sbyte)(q + dq));
        if (intra) current[mbPosition].IntraQ = q; else current[mbPosition].InterQ = q;
        return q;
    }
    private static int Median(int a, int b, int c) => a + b + c - Math.Min(a, Math.Min(b, c)) - Math.Max(a, Math.Max(b, c));
}
