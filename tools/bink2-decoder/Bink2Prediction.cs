// SPDX-License-Identifier: GPL-3.0-or-later
// Derived from the Bink2 decoder by Konstantin Shishkov (2014) and Paul B Mahol (2019).
// Upstream G source declares LGPL-2.1-or-later; combined decoder is GPL-3.0-or-later.
// Origin: sharpemu/ffmpeg-core bink2.patch at ce81eeef27fa0663bf51a22486494330852d69aa.
// See THIRD_PARTY_NOTICES.md for the retained upstream notice.
namespace OpenFallout4.Bink2;

public sealed partial class Bink2Decoder
{
    private static int Predict3(int a, int b, int c) => Math.Clamp(c + b - a, Math.Min(a, Math.Min(b, c)), Math.Max(a, Math.Max(b, c)));
    private static int Predict2(int a, int b) => Math.Min(Math.Max(a, b), Math.Max(Math.Min(a, b), 2 * a - b));
    private void PredictDc(bool luma, int mindc, int maxdc, int flags, Span<int> tdc)
    {
        int[] LTdc = previous[Math.Max(mbPosition - 1, 0)].Dc[component];
        int[] Tdc = previous[mbPosition].Dc[component];
        int[] Ldc = current[Math.Max(mbPosition - 1, 0)].Dc[component];
        int[] dc = current[mbPosition].Dc[component];
    if (luma && ((flags & 0x20) != 0) && ((flags & 0x80) != 0)) {
        dc[0]  = Math.Clamp((mindc < 0 ? 0 : 1024) + tdc[0], mindc, maxdc);
        dc[1]  = Math.Clamp(dc[0] + tdc[1], mindc, maxdc);
        dc[2]  = Math.Clamp(Predict2(dc[0], dc[1]) + tdc[2], mindc, maxdc);
        dc[3]  = Math.Clamp(Predict3(dc[0], dc[2], dc[1]) + tdc[3], mindc, maxdc);
        dc[4]  = Math.Clamp(Predict2(dc[1], dc[3]) + tdc[4], mindc, maxdc);
        dc[5]  = Math.Clamp(dc[4] + tdc[5], mindc, maxdc);
        dc[6]  = Math.Clamp(Predict3(dc[1], dc[3], dc[4]) + tdc[6], mindc, maxdc);
        dc[7]  = Math.Clamp(Predict3(dc[4], dc[6], dc[5]) + tdc[7], mindc, maxdc);
        dc[8]  = Math.Clamp(Predict2(dc[2], dc[3]) + tdc[8], mindc, maxdc);
        dc[9]  = Math.Clamp(Predict3(dc[2], dc[8], dc[3]) + tdc[9], mindc, maxdc);
        dc[10] = Math.Clamp(Predict2(dc[8], dc[9]) + tdc[10], mindc, maxdc);
        dc[11] = Math.Clamp(Predict3(dc[8], dc[10], dc[9]) + tdc[11], mindc, maxdc);
        dc[12] = Math.Clamp(Predict3(dc[3], dc[9], dc[6]) + tdc[12], mindc, maxdc);
        dc[13] = Math.Clamp(Predict3(dc[6], dc[12], dc[7]) + tdc[13], mindc, maxdc);
        dc[14] = Math.Clamp(Predict3(dc[9], dc[11], dc[12]) + tdc[14], mindc, maxdc);
        dc[15] = Math.Clamp(Predict3(dc[12], dc[14], dc[13]) + tdc[15], mindc, maxdc);
    } else if (luma && ((flags & 0x80) != 0)) {
        dc[0]  = Math.Clamp(Predict2(Ldc[5], Ldc[7]) + tdc[0], mindc, maxdc);
        dc[1]  = Math.Clamp(dc[0] + tdc[1], mindc, maxdc);
        dc[2]  = Math.Clamp(Predict3(Ldc[5], Ldc[7], dc[0]) + tdc[2], mindc, maxdc);
        dc[3]  = Math.Clamp(Predict3(dc[0], dc[2], dc[1]) + tdc[3], mindc, maxdc);
        dc[4]  = Math.Clamp(Predict2(dc[1], dc[3]) + tdc[4], mindc, maxdc);
        dc[5]  = Math.Clamp(dc[4] + tdc[5], mindc, maxdc);
        dc[6]  = Math.Clamp(Predict3(dc[1], dc[3], dc[4]) + tdc[6], mindc, maxdc);
        dc[7]  = Math.Clamp(Predict3(dc[4], dc[6], dc[5]) + tdc[7], mindc, maxdc);
        dc[8]  = Math.Clamp(Predict3(Ldc[7], Ldc[13], dc[2]) + tdc[8], mindc, maxdc);
        dc[9]  = Math.Clamp(Predict3(dc[2], dc[8], dc[3]) + tdc[9], mindc, maxdc);
        dc[10] = Math.Clamp(Predict3(Ldc[13], Ldc[15], dc[8]) + tdc[10], mindc, maxdc);
        dc[11] = Math.Clamp(Predict3(dc[8], dc[10], dc[9]) + tdc[11], mindc, maxdc);
        dc[12] = Math.Clamp(Predict3(dc[3], dc[9], dc[6]) + tdc[12], mindc, maxdc);
        dc[13] = Math.Clamp(Predict3(dc[6], dc[12], dc[7]) + tdc[13], mindc, maxdc);
        dc[14] = Math.Clamp(Predict3(dc[9], dc[11], dc[12]) + tdc[14], mindc, maxdc);
        dc[15] = Math.Clamp(Predict3(dc[12], dc[14], dc[13]) + tdc[15], mindc, maxdc);
    } else if (luma && ((flags & 0x20) != 0)) {
        dc[0]  = Math.Clamp(Predict2(Tdc[10], Tdc[11]) + tdc[0], mindc, maxdc);
        dc[1]  = Math.Clamp(Predict3(Tdc[10], dc[0], Tdc[11]) + tdc[1], mindc, maxdc);
        dc[2]  = Math.Clamp(Predict2(dc[0], dc[1]) + tdc[2], mindc, maxdc);
        dc[3]  = Math.Clamp(Predict3(dc[0], dc[2], dc[1]) + tdc[3], mindc, maxdc);
        dc[4]  = Math.Clamp(Predict3(Tdc[11], dc[1], Tdc[14]) + tdc[4], mindc, maxdc);
        dc[5]  = Math.Clamp(Predict3(Tdc[14], dc[4], Tdc[15]) + tdc[5], mindc, maxdc);
        dc[6]  = Math.Clamp(Predict3(dc[1], dc[3], dc[4]) + tdc[6], mindc, maxdc);
        dc[7]  = Math.Clamp(Predict3(dc[4], dc[6], dc[5]) + tdc[7], mindc, maxdc);
        dc[8]  = Math.Clamp(Predict2(dc[2], dc[3]) + tdc[8], mindc, maxdc);
        dc[9]  = Math.Clamp(Predict3(dc[2], dc[8], dc[3]) + tdc[9], mindc, maxdc);
        dc[10] = Math.Clamp(Predict2(dc[8], dc[9]) + tdc[10], mindc, maxdc);
        dc[11] = Math.Clamp(Predict3(dc[8], dc[10], dc[9]) + tdc[11], mindc, maxdc);
        dc[12] = Math.Clamp(Predict3(dc[3], dc[9], dc[6]) + tdc[12], mindc, maxdc);
        dc[13] = Math.Clamp(Predict3(dc[6], dc[12], dc[7]) + tdc[13], mindc, maxdc);
        dc[14] = Math.Clamp(Predict3(dc[9], dc[11], dc[12]) + tdc[14], mindc, maxdc);
        dc[15] = Math.Clamp(Predict3(dc[12], dc[14], dc[13]) + tdc[15], mindc, maxdc);
    } else if (luma) {
        dc[0]  = Math.Clamp(Predict3(LTdc[15], Ldc[5], Tdc[10]) + tdc[0], mindc, maxdc);
        dc[1]  = Math.Clamp(Predict3(Tdc[10], dc[0], Tdc[11]) + tdc[1], mindc, maxdc);
        dc[2]  = Math.Clamp(Predict3(Ldc[5], Ldc[7], dc[0]) + tdc[2], mindc, maxdc);
        dc[3]  = Math.Clamp(Predict3(dc[0], dc[2], dc[1]) + tdc[3], mindc, maxdc);
        dc[4]  = Math.Clamp(Predict3(Tdc[11], dc[1], Tdc[14]) + tdc[4], mindc, maxdc);
        dc[5]  = Math.Clamp(Predict3(Tdc[14], dc[4], Tdc[15]) + tdc[5], mindc, maxdc);
        dc[6]  = Math.Clamp(Predict3(dc[1], dc[3], dc[4]) + tdc[6], mindc, maxdc);
        dc[7]  = Math.Clamp(Predict3(dc[4], dc[6], dc[5]) + tdc[7], mindc, maxdc);
        dc[8]  = Math.Clamp(Predict3(Ldc[7], Ldc[13], dc[2]) + tdc[8], mindc, maxdc);
        dc[9]  = Math.Clamp(Predict3(dc[2], dc[8], dc[3]) + tdc[9], mindc, maxdc);
        dc[10] = Math.Clamp(Predict3(Ldc[13], Ldc[15], dc[8]) + tdc[10], mindc, maxdc);
        dc[11] = Math.Clamp(Predict3(dc[8], dc[10], dc[9]) + tdc[11], mindc, maxdc);
        dc[12] = Math.Clamp(Predict3(dc[3], dc[9], dc[6]) + tdc[12], mindc, maxdc);
        dc[13] = Math.Clamp(Predict3(dc[6], dc[12], dc[7]) + tdc[13], mindc, maxdc);
        dc[14] = Math.Clamp(Predict3(dc[9], dc[11], dc[12]) + tdc[14], mindc, maxdc);
        dc[15] = Math.Clamp(Predict3(dc[12], dc[14], dc[13]) + tdc[15], mindc, maxdc);
    } else if (!luma && ((flags & 0x20) != 0) && ((flags & 0x80) != 0)) {
        dc[0] = Math.Clamp((mindc < 0 ? 0 : 1024) + tdc[0], mindc, maxdc);
        dc[1] = Math.Clamp(dc[0] + tdc[1], mindc, maxdc);
        dc[2] = Math.Clamp(Predict2(dc[0], dc[1]) + tdc[2], mindc, maxdc);
        dc[3] = Math.Clamp(Predict3(dc[0], dc[2], dc[1]) + tdc[3], mindc, maxdc);
    } else if (!luma && ((flags & 0x80) != 0)) {
        dc[0] = Math.Clamp(Predict2(Ldc[1], Ldc[3]) + tdc[0], mindc, maxdc);
        dc[1] = Math.Clamp(dc[0] + tdc[1], mindc, maxdc);
        dc[2] = Math.Clamp(Predict3(Ldc[1], Ldc[3], dc[0]) + tdc[2], mindc, maxdc);
        dc[3] = Math.Clamp(Predict3(dc[0], dc[2], dc[1]) + tdc[3], mindc, maxdc);
    } else if (!luma && ((flags & 0x20) != 0)) {
        dc[0] = Math.Clamp(Predict2(Tdc[2], Tdc[3]) + tdc[0], mindc, maxdc);
        dc[1] = Math.Clamp(Predict3(Tdc[2], dc[0], Tdc[3]) + tdc[1], mindc, maxdc);
        dc[2] = Math.Clamp(Predict2(dc[0], dc[1]) + tdc[2], mindc, maxdc);
        dc[3] = Math.Clamp(Predict3(dc[0], dc[2], dc[1]) + tdc[3], mindc, maxdc);
    } else if (!luma) {
        dc[0] = Math.Clamp(Predict3(LTdc[3], Ldc[1], Tdc[2]) + tdc[0], mindc, maxdc);
        dc[1] = Math.Clamp(Predict3(Tdc[2], dc[0], Tdc[3]) + tdc[1], mindc, maxdc);
        dc[2] = Math.Clamp(Predict3(Ldc[1], Ldc[3], dc[0]) + tdc[2], mindc, maxdc);
        dc[3] = Math.Clamp(Predict3(dc[0], dc[2], dc[1]) + tdc[3], mindc, maxdc);
    }
}


}