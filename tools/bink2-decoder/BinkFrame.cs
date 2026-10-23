// SPDX-License-Identifier: GPL-3.0-or-later
namespace OpenFallout4.Bink2;

public sealed class BinkFrame
{
    public int Width { get; }
    public int Height { get; }
    public int CodedWidth { get; }
    public int CodedHeight { get; }
    public byte[][] Planes { get; }
    public int[] Strides { get; }

    public BinkFrame(int width, int height)
    {
        if (width <= 0 || height <= 0 || width > 4096 || height > 4096 ||
            (width & 31) != 0 || (height & 1) != 0)
            throw new NotSupportedException("Decoder frames require 32-aligned width and even height up to 4096.");
        Width = width;
        Height = height;
        CodedWidth = (width + 31) & ~31;
        CodedHeight = (height + 31) & ~31;
        Strides = [CodedWidth, CodedWidth / 2, CodedWidth / 2];
        Planes = [new byte[CodedWidth * CodedHeight],
            new byte[CodedWidth / 2 * (CodedHeight / 2)],
            new byte[CodedWidth / 2 * (CodedHeight / 2)]];
    }
}
