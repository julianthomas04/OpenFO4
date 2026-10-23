// SPDX-License-Identifier: GPL-3.0-or-later
// Derived from Shishkov (2014), Mahol (2019); upstream G source LGPL-2.1-or-later.
// Origin: sharpemu/ffmpeg-core ce81eeef27fa0663bf51a22486494330852d69aa/bink2.patch.
namespace OpenFallout4.Bink2;

public sealed partial class Bink2Decoder
{
    private static int AverageBlock(byte[] src, int offset, int stride)
    {
        int sum = 0;
        for (int i = 0; i < 8; i++)
        {
            int a = (src[offset + i] + src[offset + i + stride] + 1) >> 1;
            int b = (src[offset + i + 2 * stride] + src[offset + i + 3 * stride] + 1) >> 1;
            int c = (src[offset + i + 4 * stride] + src[offset + i + 5 * stride] + 1) >> 1;
            int e = (src[offset + i + 6 * stride] + src[offset + i + 7 * stride] + 1) >> 1;
            sum += ((((a + b + 1) >> 1) + ((c + e + 1) >> 1) + 1) >> 1);
        }
        return sum;
    }
    private static int ArithmeticMeanDc(ReadOnlySpan<byte> pixels, int stride)
    {
        int sum = 0;
        for (int y = 0; y < 8; y++)
            for (int x = 0; x < 8; x++) sum += pixels[y * stride + x];
        return (sum + 4) >> 3;
    }
    internal void AverageMacroblock(Macroblock block, int x, int y)
    {
        for (int comp = 0; comp < 3; comp++)
        {
            int plane = comp == 0 ? 0 : comp == 1 ? 2 : 1;
            int stride = output.Strides[plane], side = comp == 0 ? 4 : 2;
            int baseX = comp == 0 ? x : x / 2, baseY = comp == 0 ? y : y / 2;
            for (int i = 0; i < side * side; i++)
            {
                int raster = comp == 0 ? luma_repos[i] : i;
                int offset = (baseY + raster / side * 8) * stride + baseX + raster % side * 8;
                int treeDc = AverageBlock(output.Planes[plane], offset, stride);
                block.Dc[comp][i] = exactChromaDcAverage && comp != 0
                    ? ArithmeticMeanDc(output.Planes[plane].AsSpan(offset), stride) : treeDc;
                dcAverageDiagnostics?.Observe(frameIndex, plane, output.Planes[plane].AsSpan(offset), stride, treeDc);
                blockTrace?.Refresh(frameIndex, mbPosition, macroblockY, x / 32, y / 32, block.Type, comp, i,
                    output.Planes[plane].AsSpan(offset), stride, treeDc, block.Dc[comp][i]);
            }
        }
    }
    private static void CopyState(Macroblock src, Macroblock dst)
    {
        dst.Type = src.Type;
        for (int comp = 0; comp < 3; comp++)
        {
            src.Dc[comp].CopyTo(dst.Dc[comp], 0);
            src.State[comp].CopyTo(dst.State[comp], 0);
        }
        for (int i = 0; i < 4; i++) src.Motion[i].CopyTo(dst.Motion[i], 0);
    }
    private void CapturePrediction(int x, int y)
    {
        for (int plane = 0; plane < 3; plane++)
        {
            var (side, offset, stride) = MacroblockArea(plane, x, y);
            for (int row = 0; row < side; row++)
                output.Planes[plane].AsSpan(offset + row * stride, side).CopyTo(prediction[plane].AsSpan(row * side, side));
        }
    }
    internal void AddFilteredResidual(Macroblock block, int x, int y)
    {
        FilterMacroblockResidualEdges(block, x, y);
        for (int plane = 0; plane < 3; plane++)
        {
            var (side, offset, stride) = MacroblockArea(plane, x, y);
            for (int row = 0; row < side; row++)
                for (int col = 0; col < side; col++)
                {
                    int position = offset + row * stride + col;
                    output.Planes[plane][position] = ClipByte(prediction[plane][row * side + col] + residualPlanes[plane][position]);
                }
        }
    }
    private (int Side, int Offset, int Stride) MacroblockArea(int plane, int x, int y)
    {
        int side = plane == 0 ? 32 : 16, stride = output.Strides[plane];
        return (side, (plane == 0 ? y : y / 2) * stride + (plane == 0 ? x : x / 2), stride);
    }
    private void DecodeSlice(bool keyFrame, int start, int end)
    {
        foreach (Macroblock b in previous) b.Clear();
        foreach (Macroblock b in current) b.Clear();
        for (int y = start; y < end; y += 32)
        {
            macroblockY = y / 32;
            uint yIntra = 0, uIntra = 0, vIntra = 0, yInter = 0, uInter = 0, vInter = 0;
            int[] lru = { 2, 3, 1, 0 };
            foreach (Macroblock b in current)
            {
                b.IntraQ = b.InterQ = 0;
                foreach (int[] vector in b.Motion) Array.Clear(vector);
            }
            for (int x = 0; x < width; x += 32)
            {
                mbPosition = x / 32;
                Macroblock block = current[mbPosition];
                foreach (ushort[] values in block.State) Array.Clear(values);
                int type = block.Type = keyFrame ? 0 : DecodeType(lru);
                int flags = (y == start ? 0x80 : 0) | (x == 0 ? 0x20 : 0) | (x == 32 ? 0x200 : 0) | (x + 32 >= width ? 0x40 : 0);
                blockTrace?.Macroblock(frameIndex, "macroblock-begin", mbPosition, macroblockY, type, flags, bits.Position,
                    block.IntraQ, block.InterQ, block.Motion, block.Dc, block.State);
                vectorCount = 0;
                foreach (int[] vector in motionResidual) Array.Clear(vector);
                switch (type)
                {
                    case 0:
                        if ((flags & 0xa0) == 0 && previous[mbPosition - 1].Type != 0) AverageMacroblock(previous[mbPosition - 1], x - 32, y - 32);
                        if ((flags & 0x20) == 0 && current[mbPosition - 1].Type != 0) AverageMacroblock(current[mbPosition - 1], x - 32, y);
                        if ((flags & 0x20) != 0 && (flags & 0x80) == 0 && previous[mbPosition + 1].Type != 0) AverageMacroblock(previous[mbPosition + 1], x + 32, y - 32);
                        if ((flags & 0x80) == 0 && previous[mbPosition].Type != 0) AverageMacroblock(previous[mbPosition], x, y - 32);
                        PredictMotion(flags);
                        UpdateQ(false, 0, flags);
                        int intraQ = UpdateQ(true, DecodeDq(), flags);
                        if ((uint)intraQ >= 37) throw new InvalidDataException("Invalid intra quantizer.");
                        DecodePlanes(true, intraQ, flags, x, y, ref yIntra, ref uIntra, ref vIntra);
                        break;
                    case 1:
                        UpdateQ(false, 0, flags); UpdateQ(true, 0, flags);
                        for (int plane = 0; plane < 3; plane++)
                        {
                            int side = plane == 0 ? 32 : 16, px = plane == 0 ? x : x / 2, py = plane == 0 ? y : y / 2;
                            CopyBlock(output.Planes[plane], py * output.Strides[plane] + px, output.Strides[plane], reference.Planes[plane], py * reference.Strides[plane] + px, reference.Strides[plane], side);
                        }
                        break;
                    case 2:
                    case 3:
                        UpdateQ(true, 0, flags);
                        if (type == 2) UpdateQ(false, 0, flags);
                        DecodeMotion(); PredictMotion(flags);
                        int interQ = type == 3 ? UpdateQ(false, DecodeDq(), flags) : block.InterQ;
                        if ((uint)interQ >= 37) throw new InvalidDataException("Invalid inter quantizer.");
                        component = 0; Compensate(x, y, true);
                        component = 1; Compensate(x / 2, y / 2, false);
                        component = 2; Compensate(x / 2, y / 2, false);
                        if (blockTrace?.Matches(frameIndex, mbPosition, macroblockY) == true)
                            blockTrace.Region(frameIndex, "after-motion-compensation-before-residual", output);
                        if (type == 3)
                        {
                            CapturePrediction(x, y);
                            DecodePlanes(false, interQ, flags, x, y, ref yInter, ref uInter, ref vInter);
                            AddFilteredResidual(block, x, y);
                        }
                        break;
                    default: throw new InvalidDataException("Invalid macroblock type.");
                }
                blockTrace?.Macroblock(frameIndex, "macroblock-end", mbPosition, macroblockY, type, flags, bits.Position,
                    block.IntraQ, block.InterQ, block.Motion, block.Dc, block.State);
                if (blockTrace?.Matches(frameIndex, mbPosition, macroblockY) == true)
                    blockTrace.Region(frameIndex, "after-macroblock-decoding", output);
            }
            for (int i = 0; i < macroblockWidth; i++) CopyState(current[i], frameState[y >> 5][i]);
            (current, previous) = (previous, current);
        }
    }
    private void DecodePlanes(bool intra, int q, int flags, int x, int y, ref uint yCbp, ref uint uCbp, ref uint vCbp)
    {
        component = 0;
        DecodeComponent(true, intra, ref yCbp, q, flags, 0, y * output.Strides[0] + x, output.Strides[0]);
        if (intra || bits.ReadBit() != 0)
        {
            component = 1;
            DecodeComponent(false, intra, ref uCbp, q, flags, 2, y / 2 * output.Strides[2] + x / 2, output.Strides[2]);
            component = 2;
            DecodeComponent(false, intra, ref vCbp, q, flags, 1, y / 2 * output.Strides[1] + x / 2, output.Strides[1]);
        }
        else uCbp = vCbp = 0;
    }
}
