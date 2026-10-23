// SPDX-License-Identifier: GPL-3.0-or-later
// Derived from Shishkov (2014), Mahol (2019); upstream G source LGPL-2.1-or-later.
// Origin: sharpemu/ffmpeg-core ce81eeef27fa0663bf51a22486494330852d69aa/bink2.patch.
namespace OpenFallout4.Bink2;

public sealed partial class Bink2Decoder
{
    private static int SixTap(byte[] source, int offset, int step) =>
        (((source[offset] + source[offset + step]) * 19 >> 1)
        - (source[offset - step] + source[offset + 2 * step]) * 2
        + ((source[offset - 2 * step] + source[offset + 3 * step]) >> 1) + 8) >> 4;
    private static int SixTap(Span<short> source, int offset, int step) =>
        (((source[offset] + source[offset + step]) * 19 >> 1)
        - (source[offset - step] + source[offset + 2 * step]) * 2
        + ((source[offset - 2 * step] + source[offset + 3 * step]) >> 1) + 8) >> 4;
    private static void CopyBlock(byte[] dst, int destination, int stride, byte[] src, int source, int sourceStride, int size)
    {
        for (int row = 0; row < size; row++)
            src.AsSpan(source + row * sourceStride, size).CopyTo(dst.AsSpan(destination + row * stride, size));
    }
    private void Compensate(int x, int y, bool luma)
    {
        int plane = luma ? 0 : component == 1 ? 2 : 1;
        int stride = output.Strides[plane], sourceStride = reference.Strides[plane];
        byte[] dst = output.Planes[plane], src = reference.Planes[plane];
        int side = luma ? 16 : 8, shift = luma ? 1 : 2;
        int w = luma ? width : width / 2, h = luma ? height : height / 2;
        int[][] motion = current[mbPosition].Motion;
        for (int index = 0; index < 4; index++)
        {
            int blockX = x + (index & 1) * side, blockY = y + (index >> 1) * side;
            int mx = (motion[index][0] >> shift) + blockX;
            int my = (motion[index][1] >> shift) + blockY;
            int phaseMask = (1 << shift) - 1;
            int mode = (motion[index][0] & phaseMask) | ((motion[index][1] & phaseMask) << shift);
            int minimumX = mx, minimumY = my, maximumX = mx + side - 1, maximumY = my + side - 1;
            if (luma)
            {
                if ((mode & 1) != 0) { minimumX -= 2; maximumX += 3; }
                if ((mode & 2) != 0) { minimumY -= 2; maximumY += 3; }
            }
            else
            {
                if ((mode & 3) != 0) maximumX++;
                if ((mode >> 2) != 0) maximumY++;
            }
            int codedHeight = src.Length / sourceStride;
            if (mx < 0 || mx >= w || my < 0 || my >= h || minimumX < 0 || minimumY < 0 || maximumX >= sourceStride || maximumY >= codedHeight)
                throw new NotSupportedException($"Unimplemented motion edge extension: frame {frameIndex}, macroblock ({mbPosition}, {y / (luma ? 32 : 16)}), plane {plane}, block ({blockX}, {blockY}), vector {index}=({motion[index][0]}, {motion[index][1]}), mode {mode}, source window [{minimumX},{minimumY}]..[{maximumX},{maximumY}], coded plane {sourceStride}x{codedHeight}.");
            int destination = blockY * stride + blockX, source = my * sourceStride + mx;
            if (mode == 0) CopyBlock(dst, destination, stride, src, source, sourceStride, side);
            else if (luma) CompensateLuma(dst, destination, stride, src, source, sourceStride, mode);
            else CompensateChroma(dst, destination, stride, src, source, sourceStride, mode);
            if (index == 0)
                blockTrace?.Motion(frameIndex, mbPosition, macroblockY, plane, motion[index][0], motion[index][1], mode,
                    mx, my, minimumX, minimumY, maximumX - minimumX + 1, maximumY - minimumY + 1, src, sourceStride, dst, stride);
        }
    }
    private static void CompensateChroma(byte[] dst, int destination, int stride, byte[] src, int source, int sourceStride, int mode)
    {
        int hp = mode & 3, vp = mode >> 2;
        int shift = bink2g_chroma_shifts[hp] + bink2g_chroma_shifts[vp];
        int rounding = 1 << (shift - 1);
        for (int y = 0; y < 8; y++)
        {
            int top = source + y * sourceStride, bottom = top + (vp != 0 ? sourceStride : 0);
            for (int x = 0; x < 8; x++)
            {
                int topValue = bink2g_chroma_weights[hp][0] * src[top + x] + bink2g_chroma_weights[hp][1] * src[top + x + (hp != 0 ? 1 : 0)];
                int bottomValue = bink2g_chroma_weights[hp][0] * src[bottom + x] + bink2g_chroma_weights[hp][1] * src[bottom + x + (hp != 0 ? 1 : 0)];
                int value = bink2g_chroma_weights[vp][0] * topValue + bink2g_chroma_weights[vp][1] * bottomValue;
                dst[destination + y * stride + x] = unchecked((byte)((value + rounding) >> shift));
            }
        }
    }
    private static void CompensateLuma(byte[] dst, int destination, int stride, byte[] src, int source, int sourceStride, int mode)
    {
        if (mode == 1)
        {
            for (int y = 0; y < 16; y++)
                for (int x = 0; x < 16; x++)
                    dst[destination + y * stride + x] = ClipByte(SixTap(src, source + y * sourceStride + x, 1));
        }
        else if (mode == 2)
        {
            for (int x = 0; x < 16; x++)
                for (int y = 0; y < 16; y++)
                    dst[destination + y * stride + x] = ClipByte(SixTap(src, source + y * sourceStride + x, sourceStride));
        }
        else
        {
            Span<short> temp = stackalloc short[21 * 16];
            for (int y = 0; y < 21; y++)
                for (int x = 0; x < 16; x++)
                    temp[y * 16 + x] = unchecked((short)SixTap(src, source + (y - 2) * sourceStride + x, 1));
            for (int y = 0; y < 16; y++)
                for (int x = 0; x < 16; x++)
                    dst[destination + y * stride + x] = ClipByte(SixTap(temp, (y + 2) * 16 + x, 16));
        }
    }
}
