// SPDX-License-Identifier: GPL-3.0-or-later
// Derived from Shishkov (2014), Mahol (2019); upstream G source LGPL-2.1-or-later.
// Origin: sharpemu/ffmpeg-core ce81eeef27fa0663bf51a22486494330852d69aa/bink2.patch.
namespace OpenFallout4.Bink2;

public sealed partial class Bink2Decoder
{
    private readonly int[][] motionResidual = { new int[2], new int[2], new int[2], new int[2] };
    private int vectorCount;
    private void PredictMotion(int flags)
    {
        int[][] c_mv = current[mbPosition].Motion;
        int[][] l_mv = current[Math.Max(mbPosition - 1, 0)].Motion;
        int[][] lt_mv = previous[Math.Max(mbPosition - 1, 0)].Motion;
        int[][] t_mv = previous[mbPosition].Motion;
    if (vectorCount == 1) {
        if ((flags & 0x80) != 0) {
            if (((flags & 0x20) == 0)) {
                motionResidual[0][0] += Median(l_mv[0][0], l_mv[1][0], l_mv[3][0]);
                motionResidual[0][1] += Median(l_mv[0][1], l_mv[1][1], l_mv[3][1]);
            }
        } else {
            if (((flags & 0x20) == 0)) {
                motionResidual[0][0] += Median(lt_mv[3][0], t_mv[2][0], l_mv[1][0]);
                motionResidual[0][1] += Median(lt_mv[3][1], t_mv[2][1], l_mv[1][1]);
            } else {
                motionResidual[0][0] += Median(t_mv[0][0], t_mv[2][0], t_mv[3][0]);
                motionResidual[0][1] += Median(t_mv[0][1], t_mv[2][1], t_mv[3][1]);
            }
        }

        c_mv[0][0] = motionResidual[0][0];
        c_mv[0][1] = motionResidual[0][1];
        c_mv[1][0] = motionResidual[0][0];
        c_mv[1][1] = motionResidual[0][1];
        c_mv[2][0] = motionResidual[0][0];
        c_mv[2][1] = motionResidual[0][1];
        c_mv[3][0] = motionResidual[0][0];
        c_mv[3][1] = motionResidual[0][1];

        return;
    }

    if (((flags & 0x80) == 0)) {
        if ((flags & 0x20) != 0) {
            c_mv[0][0] = motionResidual[0][0] + Median(t_mv[0][0], t_mv[2][0], t_mv[3][0]);
            c_mv[0][1] = motionResidual[0][1] + Median(t_mv[0][1], t_mv[2][1], t_mv[3][1]);
            c_mv[1][0] = motionResidual[1][0] + Median(t_mv[2][0], t_mv[3][0], c_mv[0][0]);
            c_mv[1][1] = motionResidual[1][1] + Median(t_mv[2][1], t_mv[3][1], c_mv[0][1]);
            c_mv[2][0] = motionResidual[2][0] + Median(t_mv[2][0], c_mv[0][0], c_mv[1][0]);
            c_mv[2][1] = motionResidual[2][1] + Median(t_mv[2][1], c_mv[0][1], c_mv[1][1]);
            c_mv[3][0] = motionResidual[3][0] + Median(c_mv[0][0], c_mv[1][0], c_mv[2][0]);
            c_mv[3][1] = motionResidual[3][1] + Median(c_mv[0][1], c_mv[1][1], c_mv[2][1]);
        } else {
            c_mv[0][0] = motionResidual[0][0] + Median(t_mv[2][0], lt_mv[3][0], l_mv[1][0]);
            c_mv[0][1] = motionResidual[0][1] + Median(t_mv[2][1], lt_mv[3][1], l_mv[1][1]);
            c_mv[1][0] = motionResidual[1][0] + Median(t_mv[2][0], t_mv[3][0],  c_mv[0][0]);
            c_mv[1][1] = motionResidual[1][1] + Median(t_mv[2][1], t_mv[3][1],  c_mv[0][1]);
            c_mv[2][0] = motionResidual[2][0] + Median(l_mv[1][0], l_mv[3][0],  c_mv[0][0]);
            c_mv[2][1] = motionResidual[2][1] + Median(l_mv[1][1], l_mv[3][1],  c_mv[0][1]);
            c_mv[3][0] = motionResidual[3][0] + Median(c_mv[0][0], c_mv[1][0],  c_mv[2][0]);
            c_mv[3][1] = motionResidual[3][1] + Median(c_mv[0][1], c_mv[1][1],  c_mv[2][1]);
        }
    } else {
        if ((flags & 0x20) != 0) {
            c_mv[0][0] = motionResidual[0][0];
            c_mv[0][1] = motionResidual[0][1];
            c_mv[1][0] = motionResidual[1][0] + motionResidual[0][0];
            c_mv[1][1] = motionResidual[1][1] + motionResidual[0][1];
            c_mv[2][0] = motionResidual[2][0] + motionResidual[0][0];
            c_mv[2][1] = motionResidual[2][1] + motionResidual[0][1];
            c_mv[3][0] = motionResidual[3][0] + Median(c_mv[0][0], c_mv[1][0], c_mv[2][0]);
            c_mv[3][1] = motionResidual[3][1] + Median(c_mv[0][1], c_mv[1][1], c_mv[2][1]);
        } else {
            int left_v1 = vectorCount == 0 ? 3 : 1;

            c_mv[0][0] = motionResidual[0][0] + Median(l_mv[0][0], l_mv[1][0], l_mv[3][0]);
            c_mv[0][1] = motionResidual[0][1] + Median(l_mv[0][1], l_mv[1][1], l_mv[3][1]);
            c_mv[2][0] = motionResidual[2][0] + Median(l_mv[1][0], l_mv[3][0], c_mv[0][0]);
            c_mv[2][1] = motionResidual[2][1] + Median(l_mv[1][1], l_mv[3][1], c_mv[0][1]);
            c_mv[1][0] = motionResidual[1][0] + Median(l_mv[left_v1][0], c_mv[0][0], c_mv[2][0]);
            c_mv[1][1] = motionResidual[1][1] + Median(l_mv[left_v1][1], c_mv[0][1], c_mv[2][1]);
            c_mv[3][0] = motionResidual[3][0] + Median(c_mv[0][0], c_mv[1][0], c_mv[2][0]);
            c_mv[3][1] = motionResidual[3][1] + Median(c_mv[0][1], c_mv[1][1], c_mv[2][1]);
        }
    }
}


    private void DecodeMotion()
    {
        vectorCount = bits.ReadBit() != 0 ? 1 : 4;
        foreach (int[] v in motionResidual) Array.Clear(v);
        for (int axis = 0; axis < 2; axis++)
        {
            for (int j = 0; j < vectorCount; j++)
            {
                int value = DecodeVlc(bits, bink2g_mv_codes, bink2g_mv_bits);
                if (value >= 8 && value != 15) value -= 15;
                if (value == 15)
                {
                    int count = bits.ReadUnary(1, 12) + 4;
                    value = (int)bits.ReadBits(count) + (1 << count) - 1;
                    value = (value & 1) != 0 ? -(value >> 1) - 1 : value >> 1;
                }
                motionResidual[j][axis] = value;
            }
        }
    }
}