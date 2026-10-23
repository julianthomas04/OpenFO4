/*
 * Bink video 2 decoder
 * Copyright (c) 2014 Konstantin Shishkov
 * Copyright (c) 2019 Paul B Mahol
 *
 * This file is part of FFmpeg.
 *
 * FFmpeg is free software; you can redistribute it and/or
 * modify it under the terms of the GNU Lesser General Public
 * License as published by the Free Software Foundation; either
 * version 2.1 of the License, or (at your option) any later version.
 *
 * FFmpeg is distributed in the hope that it will be useful,
 * but WITHOUT ANY WARRANTY; without even the implied warranty of
 * MERCHANTABILITY or FITNESS FOR A PARTICULAR PURPOSE.  See the GNU
 * Lesser General Public License for more details.
 *
 * You should have received a copy of the GNU Lesser General Public
 * License along with FFmpeg; if not, write to the Free Software
 * Foundation, Inc., 51 Franklin Street, Fifth Floor, Boston, MA 02110-1301 USA
 */

namespace OpenFallout4.Bink2;

public sealed partial class Bink2Decoder
{
    internal const uint FrameFlagEdgeFilters = 0x2000;
    internal const uint FrameFlagVerticalEdgesOff = 0x4000;
    internal const uint FrameFlagHorizontalEdgesOff = 0x8000;

    internal const int BlockStateDcBits = 13;
    internal const int BlockStateDcMask = (1 << BlockStateDcBits) - 1;
    private const int BlockStateDcSignBit = 1 << (BlockStateDcBits - 1);
    internal const int BlockStateStrengthShift = BlockStateDcBits;
    internal const int BlockStateIntraFlag = 0x8000;
    private const int DcOnlyIntraState = BlockStateIntraFlag | (DcOnlyStrength << BlockStateStrengthShift);

    private bool EdgeFilterApplies(int first, int second, int component) =>
        DeringEnabled(component) && (first | (second << 2)) is not (0 or 15);

    private static (int Outer, int Inner) EdgeTaps(int delta) => ((2 * delta + 8) >> 4, (4 * delta + 8) >> 4);

    private void FilterPixelEdge(byte[] pixels, int offset, int across, int along, int first, int second, int component)
    {
        if (!EdgeFilterApplies(first, second, component)) return;
        for (var i = 0; i < 8; i++, offset += along)
        {
            var (outer, inner) = EdgeTaps(pixels[offset] - pixels[offset - across]);
            if (first >= 2) pixels[offset - 2 * across] = ClipByte(pixels[offset - 2 * across] + outer);
            if (first >= 1) pixels[offset - across] = ClipByte(pixels[offset - across] + inner);
            if (second >= 1) pixels[offset] = ClipByte(pixels[offset] - inner);
            if (second >= 2) pixels[offset + across] = ClipByte(pixels[offset + across] - outer);
        }
    }

    private void FilterResidualEdge(short[] residual, int offset, int across, int along, int first, int second, int component)
    {
        if (!EdgeFilterApplies(first, second, component)) return;
        for (var i = 0; i < 8; i++, offset += along)
        {
            var (outer, inner) = EdgeTaps(residual[offset] - residual[offset - across]);
            if (first >= 2) AddWrapping(residual, offset - 2 * across, outer);
            if (first >= 1) AddWrapping(residual, offset - across, inner);
            if (second >= 1) AddWrapping(residual, offset, -inner);
            if (second >= 2) AddWrapping(residual, offset + across, -outer);
        }
    }

    private static void AddWrapping(short[] residual, int index, int delta) =>
        residual[index] = unchecked((short)(residual[index] + delta));

    private bool DeringEnabled(int component)
    {
        if (deringFrame.HasValue && deringFrame.Value != frameIndex) return true;
        DeringComponents selected = component == 0 ? DeringComponents.Luma : DeringComponents.Chroma;
        return (deringComponents & selected) != 0;
    }

    private static int FilterStrength(ushort state) => (state >> BlockStateStrengthShift) & 3;

    private const int MotionEdgeStrength = 2;

    private static bool IsMotionForcedEdge(Macroblock a, int ai, Macroblock b, int bi, ushort leftState, ushort rightState) =>
        IsInterCoded(leftState) && IsInterCoded(rightState) &&
        (Math.Abs(a.Motion[ai][0] - b.Motion[bi][0]) > 1 || Math.Abs(a.Motion[ai][1] - b.Motion[bi][1]) > 1);

    private static bool IsInterCoded(ushort state) => (state & BlockStateIntraFlag) == 0;

    private static void SelectCodingEdge(ushort leftState, ushort rightState, ref int left, ref int right)
    {
        if ((leftState & BlockStateIntraFlag) == 0) left = 0;
        if ((rightState & BlockStateIntraFlag) == 0) right = 0;
    }

    internal static bool ReconstructionNear(ushort a, ushort b, int threshold) => Math.Abs(a - b) < threshold;

    private static int ReconstructionMultiplyHigh(int k, int value) =>
        (unchecked((short)k) * unchecked((short)value)) >> 16;

    private static int ReconstructionCurve(int basis, int left, int right, int position) => position switch
    {
        0 => basis + ReconstructionMultiplyHigh(26624, left),
        1 => basis + ReconstructionMultiplyHigh(18432, left) + (right >> 7),
        2 => basis + ReconstructionMultiplyHigh(12288, left) + (right >> 5),
        3 => basis + ReconstructionMultiplyHigh(7168, left) + (right >> 4),
        4 => basis + ReconstructionMultiplyHigh(7168, right) + (left >> 4),
        5 => basis + ReconstructionMultiplyHigh(12288, right) + (left >> 5),
        6 => basis + ReconstructionMultiplyHigh(18432, right) + (left >> 7),
        _ => basis + ReconstructionMultiplyHigh(26624, right),
    };

    private static ushort ReconstructionState(Macroblock[] row, int component, int blocksPerMacroblock,
        int blocksWide, int blockX, int blockY)
    {
        blockX = Math.Clamp(blockX, 0, blocksWide - 1);
        return row[blockX / blocksPerMacroblock].State[component]
            [blockY * blocksPerMacroblock + blockX % blocksPerMacroblock];
    }

    private static void ReconstructBlock(byte[] pixels, int offset, int stride,
        ushort left, ushort target, ushort right, ushort upLeft, ushort up, ushort upRight,
        ushort downLeft, ushort down, ushort downRight, int threshold)
    {
        if (target < DcOnlyIntraState)
            return;
        var d = target & BlockStateDcMask;
        var l = ReconstructionNear(target, left, threshold) ? left & BlockStateDcMask : d;
        var r = ReconstructionNear(target, right, threshold) ? right & BlockStateDcMask : d;
        var u = ReconstructionNear(target, up, threshold) ? up & BlockStateDcMask : d;
        var v = ReconstructionNear(target, down, threshold) ? down & BlockStateDcMask : d;
        var ul = ReconstructionNear(target, upLeft, threshold) ? upLeft & BlockStateDcMask : l;
        var ur = ReconstructionNear(target, upRight, threshold) ? upRight & BlockStateDcMask : r;
        var dl = ReconstructionNear(target, downLeft, threshold) ? downLeft & BlockStateDcMask : l;
        var dr = ReconstructionNear(target, downRight, threshold) ? downRight & BlockStateDcMask : r;
        Span<int> center = stackalloc int[8];
        Span<int> top = stackalloc int[8];
        Span<int> bottom = stackalloc int[8];
        for (var x = 0; x < 8; x++)
        {
            center[x] = ReconstructionCurve(d + 4, l - d, r - d, x);
            top[x] = ReconstructionCurve(u + 4, ul - u, ur - u, x) - center[x];
            bottom[x] = ReconstructionCurve(v + 4, dl - v, dr - v, x) - center[x];
        }
        for (var y = 0; y < 8; y++, offset += stride)
            for (var x = 0; x < 8; x++)
                pixels[offset + x] = ClipByte(ReconstructionCurve(center[x], top[x], bottom[x], y) >> 3);
    }

    private void ReconstructBlocks(byte[] pixels, int offset, int stride, int component, int globalY,
        Macroblock[] upRow, int upBy, Macroblock[] row, int by, Macroblock[] downRow, int downBy)
    {
        var chroma = component is 1 or 2;
        var blocksPerMacroblock = chroma ? 2 : 4;
        var codedBlocksWide = macroblockWidth * blocksPerMacroblock;
        var codedPlaneHeight = ((height + 31) >> 5) * blocksPerMacroblock * 8;
        var threshold = (int)(frameFlags & 0xFF) << 3;
        var haveUp = globalY >= 8;
        var haveDown = globalY + 8 < codedPlaneHeight;
        for (var bx = 0; bx < codedBlocksWide; bx++)
        {
            var target = ReconstructionState(row, component, blocksPerMacroblock, codedBlocksWide, bx, by);
            var left = bx != 0 ? ReconstructionState(row, component, blocksPerMacroblock, codedBlocksWide, bx - 1, by) : target;
            var right = bx + 1 < codedBlocksWide ? ReconstructionState(row, component, blocksPerMacroblock, codedBlocksWide, bx + 1, by) : target;
            var upLeft = haveUp && bx != 0 ? ReconstructionState(upRow, component, blocksPerMacroblock, codedBlocksWide, bx - 1, upBy) : (ushort)0;
            var up = haveUp ? ReconstructionState(upRow, component, blocksPerMacroblock, codedBlocksWide, bx, upBy) : target;
            var upRight = haveUp && bx + 1 < codedBlocksWide ? ReconstructionState(upRow, component, blocksPerMacroblock, codedBlocksWide, bx + 1, upBy) : (ushort)0;
            var downLeft = haveDown && bx != 0 ? ReconstructionState(downRow, component, blocksPerMacroblock, codedBlocksWide, bx - 1, downBy) : (ushort)0;
            var down = haveDown ? ReconstructionState(downRow, component, blocksPerMacroblock, codedBlocksWide, bx, downBy) : target;
            var downRight = haveDown && bx + 1 < codedBlocksWide ? ReconstructionState(downRow, component, blocksPerMacroblock, codedBlocksWide, bx + 1, downBy) : (ushort)0;
            ReconstructBlock(pixels, offset + bx * 8, stride,
                left, target, right, upLeft, up, upRight, downLeft, down, downRight, threshold);
        }
    }

    private void ReconstructRow(int[] offsets, int y, Macroblock[] previous, Macroblock[] current, bool last)
    {
        for (var component = 0; component < 3; component++)
        {
            var chroma = component is 1 or 2;
            var plane = component == 1 ? 2 : component == 2 ? 1 : component;
            var blocksPerMacroblock = chroma ? 2 : 4;
            var planeY = chroma ? y >> 1 : y;
            var stride = output.Strides[plane];
            var pixels = output.Planes[plane];
            if (y > 0)
            {
                var by = blocksPerMacroblock - 1;
                ReconstructBlocks(pixels, offsets[plane] - 8 * stride, stride, component, planeY - 8,
                    previous, by - 1, previous, by, current, 0);
            }
            for (var by = 0; by < blocksPerMacroblock - 1; by++)
            {
                var upRow = by != 0 ? current : y > 0 ? previous : current;
                var upBy = by != 0 ? by - 1 : y > 0 ? blocksPerMacroblock - 1 : 0;
                ReconstructBlocks(pixels, offsets[plane] + by * 8 * stride, stride, component, planeY + by * 8,
                    upRow, upBy, current, by, current, by + 1);
            }
            if (last)
            {
                var by = blocksPerMacroblock - 1;
                ReconstructBlocks(pixels, offsets[plane] + by * 8 * stride, stride, component, planeY + by * 8,
                    current, by - 1, current, by, current, by);
            }
        }
    }

    private void FilterVerticalRow(int[] offsets, int component, Macroblock[] row, int by, int relativeY)
    {
        if ((frameFlags & FrameFlagVerticalEdgesOff) != 0)
            return;
        var chroma = component != 0;
        var plane = component == 1 ? 2 : component == 2 ? 1 : 0;
        var blocksPerMacroblock = chroma ? 2 : 4;
        var planeWidth = chroma ? (width + 1) >> 1 : width;
        var blocksWide = (planeWidth + 7) >> 3;
        for (var bx = 1; bx < blocksWide; bx++)
        {
            var mb = bx / blocksPerMacroblock;
            var localX = bx % blocksPerMacroblock;
            var leftMb = localX != 0 ? mb : mb - 1;
            var leftX = localX != 0 ? localX - 1 : blocksPerMacroblock - 1;
            var leftState = row[leftMb].State[component][by * blocksPerMacroblock + leftX];
            var rightState = row[mb].State[component][by * blocksPerMacroblock + localX];
            var left = FilterStrength(leftState);
            var right = FilterStrength(rightState);
            var edgeX = bx * (chroma ? 16 : 8);
            var residualEdge = localX != 0 && IsInterCoded(leftState) && IsInterCoded(rightState);
            if (!residualEdge) SelectCodingEdge(leftState, rightState, ref left, ref right);
            if (edgeX >= width)
                continue;
            var qrow = chroma ? by : by >> 1;
            var leftQ = 2 * qrow + (localX != 0 ? 0 : 1);
            var rightQ = 2 * qrow + (localX != 0 ? 1 : 0);
            var motionForced = (chroma || (bx & 1) == 0) &&
                IsMotionForcedEdge(row[leftMb], leftQ, row[mb], rightQ, leftState, rightState);
            var offset = offsets[plane] + bx * 8 + relativeY * output.Strides[plane];
            if (motionForced)
                FilterPixelEdge(output.Planes[plane], offset, 1, output.Strides[plane], MotionEdgeStrength, MotionEdgeStrength, component);
            else if (!residualEdge)
                FilterPixelEdge(output.Planes[plane], offset, 1, output.Strides[plane], left, right, component);
        }
    }

    private void FilterHorizontalRow(int[] offsets, int component, Macroblock[] topRow, int topBy,
        Macroblock[] bottomRow, int bottomBy, int relativeY, int edgeY)
    {
        if ((frameFlags & FrameFlagHorizontalEdgesOff) != 0 || edgeY == 0)
            return;
        var chroma = component != 0;
        var plane = component == 1 ? 2 : component == 2 ? 1 : 0;
        var blocksPerMacroblock = chroma ? 2 : 4;
        var planeWidth = chroma ? (width + 1) >> 1 : width;
        var blocksWide = (planeWidth + 7) >> 3;
        for (var bx = 0; bx < blocksWide; bx++)
        {
            var mb = bx / blocksPerMacroblock;
            var localX = bx % blocksPerMacroblock;
            var topState = topRow[mb].State[component][topBy * blocksPerMacroblock + localX];
            var bottomState = bottomRow[mb].State[component][bottomBy * blocksPerMacroblock + localX];
            var top = FilterStrength(topState);
            var bottom = FilterStrength(bottomState);
            var residualEdge = bottomBy != 0 && IsInterCoded(topState) && IsInterCoded(bottomState);
            if (!residualEdge) SelectCodingEdge(topState, bottomState, ref top, ref bottom);
            var qcol = chroma ? localX : localX >> 1;
            var topQ = (bottomBy != 0 ? 0 : 2) + qcol;
            var bottomQ = (bottomBy != 0 ? 2 : 0) + qcol;
            var motionForced = (chroma || bottomBy is 0 or 2) &&
                IsMotionForcedEdge(topRow[mb], topQ, bottomRow[mb], bottomQ, topState, bottomState);
            var offset = offsets[plane] + bx * 8 + relativeY * output.Strides[plane];
            if (motionForced)
                FilterPixelEdge(output.Planes[plane], offset, output.Strides[plane], 1, MotionEdgeStrength, MotionEdgeStrength, component);
            else if (!residualEdge)
                FilterPixelEdge(output.Planes[plane], offset, output.Strides[plane], 1, top, bottom, component);
        }
    }

    private const int DcOnlyStrength = 3;
    // Measured against reference frames: a limit of 31 or 33 each mismatches.
    private const int MaximumSmoothDcOnlyStep = 32;

    private static int StateDc(ushort state) => ((state & BlockStateDcMask) ^ BlockStateDcSignBit) - BlockStateDcSignBit;

    internal static (int First, int Second) ResidualEdgeStrengths(ushort first, ushort second)
    {
        int a = FilterStrength(first), b = FilterStrength(second);
        if (a == DcOnlyStrength && b == DcOnlyStrength && Math.Abs(StateDc(first) - StateDc(second)) > MaximumSmoothDcOnlyStep)
            return (MotionEdgeStrength, MotionEdgeStrength);
        return (a, b);
    }

    private void FilterMacroblockResidualEdges(Macroblock block, int x, int y)
    {
        if ((frameFlags & FrameFlagEdgeFilters) == 0)
            return;
        for (var component = 0; component < 3; component++)
        {
            var chroma = component != 0;
            var plane = component == 1 ? 2 : component == 2 ? 1 : 0;
            var blocksPerMacroblock = chroma ? 2 : 4;
            var stride = output.Strides[plane];
            var origin = (chroma ? y >> 1 : y) * stride + (chroma ? x >> 1 : x);
            ushort[] states = block.State[component];
            if ((frameFlags & FrameFlagVerticalEdgesOff) == 0)
                for (var by = 0; by < blocksPerMacroblock; by++)
                    for (var bx = 1; bx < blocksPerMacroblock; bx++)
                    {
                        var (left, right) = ResidualEdgeStrengths(states[by * blocksPerMacroblock + bx - 1], states[by * blocksPerMacroblock + bx]);
                        FilterResidualEdge(residualPlanes[plane], origin + by * 8 * stride + bx * 8, 1, stride, left, right, component);
                    }
            if ((frameFlags & FrameFlagHorizontalEdgesOff) == 0)
                for (var by = 1; by < blocksPerMacroblock; by++)
                    for (var bx = 0; bx < blocksPerMacroblock; bx++)
                    {
                        var (top, bottom) = ResidualEdgeStrengths(states[(by - 1) * blocksPerMacroblock + bx], states[by * blocksPerMacroblock + bx]);
                        FilterResidualEdge(residualPlanes[plane], origin + by * 8 * stride + bx * 8, stride, 1, top, bottom, component);
                    }
        }
    }

    private void PostprocessRow(int[] offsets, int y, int frameEnd, Macroblock[] previous, Macroblock[] current)
    {
        var last = y + 32 >= frameEnd;
        ReconstructRow(offsets, y, previous, current, last);
        if (blockTrace?.Matches(frameIndex, blockTrace.MacroblockX, y / 32) == true)
            blockTrace.Region(frameIndex, "after-selected-row-reconstruction-before-row-filters", output);
        for (var component = 0; component < 3; component++)
        {
            var blocksPerMacroblock = component != 0 ? 2 : 4;
            if (y > 0)
                FilterVerticalRow(offsets, component, previous, blocksPerMacroblock - 1, -8);
            for (var by = 0; by < blocksPerMacroblock - 1; by++)
                FilterVerticalRow(offsets, component, current, by, by * 8);
            if (last)
                FilterVerticalRow(offsets, component, current, blocksPerMacroblock - 1, (blocksPerMacroblock - 1) * 8);
        }
        if (y > 0)
        {
            FilterHorizontalRow(offsets, 0, previous, 2, previous, 3, -8, y - 8);
            FilterHorizontalRow(offsets, 0, previous, 3, current, 0, 0, y);
            for (var component = 1; component <= 2; component++)
            {
                FilterHorizontalRow(offsets, component, previous, 0, previous, 1, -8, y - 16);
                FilterHorizontalRow(offsets, component, previous, 1, current, 0, 0, y);
            }
        }
        FilterHorizontalRow(offsets, 0, current, 0, current, 1, 8, y + 8);
        FilterHorizontalRow(offsets, 0, current, 1, current, 2, 16, y + 16);
        if (last)
        {
            FilterHorizontalRow(offsets, 0, current, 2, current, 3, 24, y + 24);
            for (var component = 1; component <= 2; component++)
                FilterHorizontalRow(offsets, component, current, 0, current, 1, 8, y + 16);
        }
    }

    internal void PostprocessFrame()
    {
        var macroblockHeight = (height + 31) >> 5;
        var frameEnd = macroblockHeight << 5;
        for (var mbY = 0; mbY < macroblockHeight; mbY++)
        {
            var y = mbY << 5;
            int[] offsets = [y * output.Strides[0], (y >> 1) * output.Strides[1], (y >> 1) * output.Strides[2]];
            PostprocessRow(offsets, y, frameEnd, frameState[mbY != 0 ? mbY - 1 : 0], frameState[mbY]);
        }
    }
}
