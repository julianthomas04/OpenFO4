// SPDX-License-Identifier: GPL-3.0-or-later
// Derived from the Bink2 decoder by Konstantin Shishkov (2014) and Paul B Mahol (2019).
// Upstream G source declares LGPL-2.1-or-later; combined decoder is GPL-3.0-or-later.
// Origin: sharpemu/ffmpeg-core bink2.patch at ce81eeef27fa0663bf51a22486494330852d69aa.
// See THIRD_PARTY_NOTICES.md for the retained upstream notice.
namespace OpenFallout4.Bink2;

public sealed partial class Bink2Decoder
{
    private static int MulA(int v) => v + (v >> 2);
    private static int MulB(int v) => v >> 1;
    private static int MulC(int v) => v - (v >> 2) - (v >> 4);
    private static int MulD(int v) => v + (v >> 2) - (v >> 4);
    private static int MulE(int v) => v >> 2;
    private static byte ClipByte(int v) => (byte)Math.Clamp(v, 0, 255);
    internal static void Idct1D(Span<short> blk, int step, int shift)
    {
    int tmp00 =  blk[3*step] + blk[5*step];
    int tmp01 =  blk[3*step] - blk[5*step];
    int tmp02 =  MulA(blk[2*step]) + MulB(blk[6*step]);
    int tmp03 =  MulB(blk[2*step]) - MulA(blk[6*step]);
    int tmp0  = (blk[0*step] + blk[4*step]) + tmp02;
    int tmp1  = (blk[0*step] + blk[4*step]) - tmp02;
    int tmp2  =  blk[0*step] - blk[4*step];
    int tmp3  =  blk[1*step] + tmp00;
    int tmp4  =  blk[1*step] - tmp00;
    int tmp5  =  tmp01 + blk[7*step];
    int tmp6  =  tmp01 - blk[7*step];
    int tmp7  =  tmp4 + MulC(tmp6);
    int tmp8  =  MulC(tmp4) - tmp6;
    int tmp9  =  MulD(tmp3) + MulE(tmp5);
    int tmp10 =  MulE(tmp3) - MulD(tmp5);
    int tmp11 =  tmp2 + tmp03;
    int tmp12 =  tmp2 - tmp03;

    blk[0*step] = unchecked((short)((tmp0  + tmp9)  >> shift));
    blk[1*step] = unchecked((short)((tmp11 + tmp7)  >> shift));
    blk[2*step] = unchecked((short)((tmp12 + tmp8)  >> shift));
    blk[3*step] = unchecked((short)((tmp1  + tmp10) >> shift));
    blk[4*step] = unchecked((short)((tmp1  - tmp10) >> shift));
    blk[5*step] = unchecked((short)((tmp12 - tmp8)  >> shift));
    blk[6*step] = unchecked((short)((tmp11 - tmp7)  >> shift));
    blk[7*step] = unchecked((short)((tmp0  - tmp9)  >> shift));
}


    private static void InverseTransform(Span<short> block)
    {
        for (int i = 0; i < 8; i++) Idct1D(block[i..], 8, 0);
        for (int i = 0; i < 8; i++) Idct1D(block[(i * 8)..], 1, 6);
    }

    internal static void IdctIntra(Span<byte> dst, int stride, Span<short> block)
    {
        InverseTransform(block);
        for (int i = 0; i < 8; i++)
            for (int j = 0; j < 8; j++)
                dst[i * stride + j] = ClipByte(block[j * 8 + i]);
    }

    internal static void IdctResidual(Span<short> residual, int stride, Span<short> block)
    {
        InverseTransform(block);
        for (int i = 0; i < 8; i++)
            for (int j = 0; j < 8; j++)
                residual[i * stride + j] = block[j * 8 + i];
    }
}
