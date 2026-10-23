// SPDX-License-Identifier: GPL-3.0-or-later
using System.Text.Json;

namespace OpenFallout4.Bink2;

internal sealed class BlockTrace
{
    private static readonly int[] LumaLeftTopIndices = { 15 };
    private static readonly int[] ChromaLeftTopIndices = { 3 };
    private static readonly int[] LumaTopIndices = { 10, 11 };
    private static readonly int[] ChromaTopIndices = { 2 };
    private static readonly int[] LumaLeftIndices = { 5, 7 };
    private static readonly int[] ChromaLeftIndices = { 1 };
    private readonly TextWriter writer;
    private readonly HashSet<int> frames;
    internal int MacroblockX { get; }
    internal int MacroblockY { get; }
    internal BlockTrace(TextWriter writer, int macroblockX, int macroblockY, int[] sourceFrames)
    {
        ArgumentNullException.ThrowIfNull(writer);
        ArgumentNullException.ThrowIfNull(sourceFrames);
        ArgumentOutOfRangeException.ThrowIfNegative(macroblockX);
        ArgumentOutOfRangeException.ThrowIfNegative(macroblockY);
        if (sourceFrames.Length is < 1 or > 3 || sourceFrames.Any(f => f < 0) || sourceFrames.Distinct().Count() != sourceFrames.Length)
            throw new ArgumentException("Trace requires one through three distinct nonnegative source frames.", nameof(sourceFrames));
        this.writer = writer;
        frames = new HashSet<int>(sourceFrames);
        MacroblockX = macroblockX; MacroblockY = macroblockY;
    }
    internal void ValidateDimensions(int width, int height)
    {
        if ((long)MacroblockX * 32 + 16 > width || (long)MacroblockY * 32 + 16 > height)
            throw new ArgumentException("Trace ROI must fit the visible image.");
    }
    internal bool Matches(int frame, int x, int y) => frames.Contains(frame) && x == MacroblockX && y == MacroblockY;
    private void Write(int frame, string stage, object data) => writer.WriteLine(JsonSerializer.Serialize(new
    { frame, stage, macroblockX = MacroblockX, macroblockY = MacroblockY, data }));
    private static int[] Pixels(byte[] pixels, int stride, int x, int y, int width, int height)
    {
        int[] result = new int[width * height];
        for (int row = 0; row < height; row++)
            for (int col = 0; col < width; col++) result[row * width + col] = pixels[(y + row) * stride + x + col];
        return result;
    }
    private static object Samples(int[] pixels, int x, int y, int width, int height) => new
    { x, y, width, height, mean = pixels.Average(), minimum = pixels.Min(), maximum = pixels.Max(), pixels };
    internal void Region(int frame, string stage, BinkFrame image)
    {
        if (!frames.Contains(frame)) return;
        object[] planes = new object[3];
        for (int plane = 0; plane < 3; plane++)
        {
            int side = plane == 0 ? 16 : 8;
            int x = MacroblockX * (plane == 0 ? 32 : 16), y = MacroblockY * (plane == 0 ? 32 : 16);
            planes[plane] = new { plane, name = plane == 0 ? "Y" : plane == 1 ? "U" : "V",
                samples = Samples(Pixels(image.Planes[plane], image.Strides[plane], x, y, side, side), x, y, side, side) };
        }
        Write(frame, stage, new { planes });
    }
    internal void Macroblock(int frame, string stage, int x, int y, int type, int flags, int position,
        int intraQ, int interQ, int[][] motion, int[][] dc, ushort[][] state)
    {
        if (x == MacroblockX && y == MacroblockY && stage == "macroblock-end")
            Write(frame, "macroblock-history", new { type, flags, bitPosition = position, intraQ, interQ });
        if (!Matches(frame, x, y)) return;
        Write(frame, stage, new { type, flags, bitPosition = position, intraQ, interQ, motion,
            selectedDc = dc.Select((a, c) => a.Take(c == 0 ? 4 : 1).ToArray()).ToArray(),
            selectedState = state.Select((a, c) => (c == 0 ? new[] { a[0], a[1], a[4], a[5] } : new[] { a[0] })).ToArray(),
            componentPlaneOrder = "component0:Y, component1:V(plane2), component2:U(plane1)" });
    }
    internal void DcInput(int frame, int x, int y, int component, int q, int flags, int position,
        ReadOnlySpan<int> residual, int[] leftTop, int[] top, int[] left, int leftTopType, int topType, int leftType)
    {
        if (!Matches(frame, x, y)) return;
        bool luma = component == 0;
        Write(frame, "dc-input", new { component, plane = component == 1 ? 2 : component == 2 ? 1 : 0,
            q, flags, bitPosition = position, residual = residual[..(luma ? 16 : 4)].ToArray(),
            leftTop = new { valid = (flags & 0xa0) == 0, type = leftTopType, indices = luma ? LumaLeftTopIndices : ChromaLeftTopIndices, values = luma ? new[] { leftTop[15] } : new[] { leftTop[3] } },
            top = new { valid = (flags & 0x80) == 0, type = topType, indices = luma ? LumaTopIndices : ChromaTopIndices, values = luma ? new[] { top[10], top[11] } : new[] { top[2] } },
            left = new { valid = (flags & 0x20) == 0, type = leftType, indices = luma ? LumaLeftIndices : ChromaLeftIndices, values = luma ? new[] { left[5], left[7] } : new[] { left[1] } } });
    }
    internal void DcOutput(int frame, int x, int y, int component, int[] dc)
    {
        if (Matches(frame, x, y)) Write(frame, "dc-output", new { component, dc = dc.Take(component == 0 ? 4 : 1).ToArray() });
    }
    internal void Refresh(int frame, int callerX, int callerY, int targetX, int targetY, int targetType,
        int component, int index, ReadOnlySpan<byte> input, int stride, int stagedDc, int usedDc)
    {
        bool caller = Matches(frame, callerX, callerY), target = Matches(frame, targetX, targetY);
        if (!caller && !target) return;
        int dx = targetX - callerX, dy = targetY - callerY;
        bool consumed = dx == -1 && dy == -1 ? index == (component == 0 ? 15 : 3)
            : dx == -1 && dy == 0 ? component == 0 ? index is 5 or 7 : index == 1
            : dx == 0 && dy == -1 && (component == 0 ? index is 10 or 11 : index == 2);
        if (!consumed) return;
        int[] pixels = new int[64]; int sum = 0;
        for (int y = 0; y < 8; y++) for (int x = 0; x < 8; x++) { int value = input[y * stride + x]; pixels[y * 8 + x] = value; sum += value; }
        Write(frame, "neighbor-refresh", new { callerX, callerY, targetX, targetY, callerSelected = caller, targetSelected = target, targetType, component,
            plane = component == 1 ? 2 : component == 2 ? 1 : 0, index, sum64 = sum, stagedDc, exactDc = (sum + 4) >> 3, usedDc, pixels });
    }
    internal void Coefficients(int frame, int mbX, int mbY, int component, int index, int raster, int dc, int acCount, short[] coefficients)
    {
        if (!Matches(frame, mbX, mbY) || index >= (component == 0 ? 4 : 1)) return;
        Write(frame, "idct-input-coefficients", new { component, index, raster, dc, acCount,
            coefficients = coefficients.Select(v => (int)v).ToArray() });
    }
    internal void Motion(int frame, int mbX, int mbY, int plane, int vectorX, int vectorY, int mode,
        int sourceX, int sourceY, int windowX, int windowY, int windowWidth, int windowHeight,
        byte[] source, int sourceStride, byte[] destination, int destinationStride)
    {
        if (!Matches(frame, mbX, mbY)) return;
        int side = plane == 0 ? 16 : 8;
        int destinationX = MacroblockX * (plane == 0 ? 32 : 16), destinationY = MacroblockY * (plane == 0 ? 32 : 16);
        Write(frame, "motion-compensated-block", new { plane, referenceFrame = frame - 1, vectorX, vectorY, mode, sourceX, sourceY,
            referenceWindow = Samples(Pixels(source, sourceStride, windowX, windowY, windowWidth, windowHeight), windowX, windowY, windowWidth, windowHeight),
            compensated = Samples(Pixels(destination, destinationStride, destinationX, destinationY, side, side), destinationX, destinationY, side, side) });
    }
}
