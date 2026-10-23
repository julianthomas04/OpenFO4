// SPDX-License-Identifier: GPL-3.0-or-later
// Derived from the Bink2 decoder by Konstantin Shishkov (2014) and Paul B Mahol (2019).
// Upstream G source declares LGPL-2.1-or-later; combined decoder is GPL-3.0-or-later.
// Origin: sharpemu/ffmpeg-core bink2.patch at ce81eeef27fa0663bf51a22486494330852d69aa.
// See THIRD_PARTY_NOTICES.md for the retained upstream notice.
namespace OpenFallout4.Bink2;

public sealed partial class Bink2Decoder
{
    private readonly short[][] blocks = { new short[64], new short[64], new short[64], new short[64] };
    private readonly int[] acCounts = new int[4];
    private static int DecodeVlc(BinkBitReader bits, int[] codes, int[] lengths)
    {
        uint code = 0;
        int max = lengths.Max();
        for (int length = 1; length <= max; length++)
        {
            code |= (uint)bits.ReadBit() << (length - 1);
            for (int symbol = 0; symbol < lengths.Length; symbol++)
                if (lengths[symbol] == length && codes[symbol] == code) return symbol;
        }
        throw new InvalidDataException("Invalid Bink2 VLC code.");
    }
    private int DecodeType(int[] lru)
    {
        int index = bits.ReadUnary(1, 3);
        int value = lru[index == 2 ? 3 : index == 3 ? 2 : index];
        if (index == 1) (lru[0], lru[1]) = (lru[1], lru[0]);
        if (index == 2) (lru[2], lru[3]) = (lru[3], lru[2]);
        if (index == 3) (lru[1], lru[2]) = (lru[2], lru[1]);
        return value;
    }
    private int DecodeDq()
    {
        int dq = bits.ReadUnary(1, 4);
        if (dq == 3) dq += bits.ReadBit();
        else if (dq == 4) dq += (int)bits.ReadBits(5) + 1;
        if (dq != 0 && bits.ReadBit() != 0) dq = -dq;
        return dq;
    }
    private uint DecodeLumaCbp(uint previousCbp)
    {
        int ones = System.Numerics.BitOperations.PopCount(previousCbp & 0xffff);
        uint mask = 0, cbp = 0;
        if (ones > 7) { ones = 16 - ones; mask = 0xffff; }
        if (bits.ReadBit() == 0)
        {
            if (ones < 4)
            {
                for (int j = 0; j < 16; j += 4)
                    if (bits.ReadBit() == 0) cbp |= bits.ReadBits(4) << j;
            }
            else cbp = bits.ReadBits(16);
        }
        cbp ^= mask;
        if ((frameFlags & 0x40000) == 0 || cbp != 0)
            if (bits.ReadBit() != 0) cbp |= cbp << 16;
        return cbp;
    }
    private uint DecodeChromaCbp(uint previousCbp)
    {
        uint cbp = previousCbp & 0xf0000 | (uint)bink2g_chroma_cbp_pat[previousCbp & 15];
        if (bits.ReadBit() == 0)
        {
            cbp = bits.ReadBits(4);
            if (bits.ReadBit() != 0) cbp |= cbp << 16;
        }
        return cbp;
    }
    private void DecodeDc(bool luma, int q, int minimum, int maximum, int flags)
    {
        Span<int> residual = stackalloc int[16];
        residual.Clear();
        int pattern = bink2g_dc_pat[Math.Max(q, 8)];
        if (bits.ReadBit() != 0)
        {
            for (int i = 0; i < (luma ? 16 : 4); i++)
            {
                int count = bits.ReadUnary(0, 12);
                if (count > 3) count = (1 << (count - 3)) + (int)bits.ReadBits(count - 3) + 2;
                if (count != 0 && bits.ReadBit() != 0) count = -count;
                residual[i] = unchecked((count * pattern + 0x1ff) >> 10);
            }
        }
        if (blockTrace?.Matches(frameIndex, mbPosition, macroblockY) == true)
        {
            int neighbor = Math.Max(mbPosition - 1, 0);
            blockTrace.DcInput(frameIndex, mbPosition, macroblockY, component, q, flags, bits.Position, residual,
                previous[neighbor].Dc[component], previous[mbPosition].Dc[component], current[neighbor].Dc[component],
                previous[neighbor].Type, previous[mbPosition].Type, current[neighbor].Type);
        }
        PredictDc(luma, minimum, maximum, flags, residual);
        blockTrace?.DcOutput(frameIndex, mbPosition, macroblockY, component, current[mbPosition].Dc[component]);
    }
    private void DecodeAc(uint cbp, int q, int[][] qmat)
    {
        for (int i = 0; i < 4; i++) { Array.Clear(blocks[i]); acCounts[i] = 0; }
        if ((cbp & 15) == 0) return;
        int table = (cbp & 0xffff0000) != 0 ? 1 : 0;
        for (int i = 0; i < 4; i++, cbp >>= 1)
        {
            if ((cbp & 1) == 0) continue;
            int next = 0, index = 1;
            while (index < 64)
            {
                next--;
                if (next < 1)
                {
                    int skip = DecodeVlc(bits, bink2g_ac_skip_codes[table], bink2g_ac_skip_bits[table]);
                    next = bink2_next_skips[skip];
                    skip = bink2g_skips[skip];
                    if (skip == 11) skip = (int)bits.ReadBits(6);
                    index += skip;
                    if (index >= 64) break;
                }
                int value = bits.ReadUnary(0, 12) + 1;
                if (value > 3) value = (int)bits.ReadBits(value - 3) + (1 << (value - 3)) + 2;
                if (bits.ReadBit() != 0) value = -value;
                int scan = bink2g_scan[index];
                blocks[i][scan] = unchecked((short)((value * qmat[q & 3][scan] * (1 << (q >> 2)) + 64) >> 7));
                acCounts[i]++;
                index++;
            }
        }
    }
    private static ushort BlockState(int dc, int count, bool intra)
    {
        int strength = (count < 8 ? 1 : 0) + (count < 4 ? 1 : 0) + (count == 0 ? 1 : 0);
        return (ushort)((dc & BlockStateDcMask) | strength << BlockStateStrengthShift | (intra ? BlockStateIntraFlag : 0));
    }
    private void DecodeComponent(bool luma, bool intra, ref uint previousCbp, int q, int flags, int plane, int offset, int stride)
    {
        uint cbp = previousCbp = luma ? DecodeLumaCbp(previousCbp) : DecodeChromaCbp(previousCbp);
        int maximum = intra || (frameFlags & 0x40000) != 0 ? 2047 : 1023;
        DecodeDc(luma, q, intra ? 0 : -maximum, maximum, intra ? flags : 0xa8);
        int[] dc = current[mbPosition].Dc[component];
        ushort[] state = current[mbPosition].State[component];
        int[][] matrix = intra ? (luma ? bink2g_luma_intra_qmat : bink2g_chroma_intra_qmat) : bink2g_inter_qmat;
        for (int group = 0; group < (luma ? 4 : 1); group++)
        {
            DecodeAc(cbp >> (4 * group), q, matrix);
            for (int j = 0; j < 4; j++)
            {
                int index = group * 4 + j;
                int raster = luma ? luma_repos[index] : index;
                state[raster] = BlockState(dc[index], acCounts[j], intra);
                blocks[j][0] = unchecked((short)(dc[index] * 8 + 32));
                blockTrace?.Coefficients(frameIndex, mbPosition, macroblockY, component, index, raster, dc[index], acCounts[j], blocks[j]);
                int side = luma ? 4 : 2;
                int blockOffset = offset + raster % side * 8 + raster / side * 8 * stride;
                if (intra)
                    IdctIntra(output.Planes[plane].AsSpan(blockOffset), stride, blocks[j]);
                else
                    IdctResidual(residualPlanes[plane].AsSpan(blockOffset), stride, blocks[j]);
            }
        }
    }
}
