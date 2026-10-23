// SPDX-License-Identifier: GPL-3.0-or-later
namespace OpenFallout4.Bink2;

internal sealed record DcAveragePlaneStatistics(
    string Plane, long Calls, long TreeDcSum, long ExactSum64, long ScaledBiasSum,
    double MeanDcBias, double MeanPixelEquivalentBias, int? MinimumScaledBias, int? MaximumScaledBias,
    IReadOnlyDictionary<int, long> ScaledBiasHistogram, long[] FirstLevelTieHistogram,
    long[] SecondLevelTieHistogram, long[] ThirdLevelTieHistogram);

internal sealed record DcAverageSnapshot(int FinalSourceFrame, string Weighting,
    DcAveragePlaneStatistics[] Overall, DcAveragePlaneStatistics[] FinalFrame);

internal sealed class DcAverageDiagnostics
{
    private sealed class Accumulator
    {
        internal long Calls, TreeDcSum, ExactSum64, ScaledBiasSum;
        internal int? MinimumScaledBias, MaximumScaledBias;
        internal readonly SortedDictionary<int, long> BiasHistogram = new();
        internal readonly long[] FirstTies = new long[33], SecondTies = new long[17], ThirdTies = new long[9];
        internal void Add(int treeDc, int exactSum, int firstTies, int secondTies, int thirdTies)
        {
            int bias = 8 * treeDc - exactSum;
            Calls++; TreeDcSum += treeDc; ExactSum64 += exactSum; ScaledBiasSum += bias;
            MinimumScaledBias = Math.Min(MinimumScaledBias ?? bias, bias);
            MaximumScaledBias = Math.Max(MaximumScaledBias ?? bias, bias);
            BiasHistogram[bias] = BiasHistogram.GetValueOrDefault(bias) + 1;
            FirstTies[firstTies]++; SecondTies[secondTies]++; ThirdTies[thirdTies]++;
        }
        internal DcAveragePlaneStatistics Snapshot(string plane) => new(plane, Calls, TreeDcSum, ExactSum64,
            ScaledBiasSum, Calls == 0 ? 0 : ScaledBiasSum / (8.0 * Calls),
            Calls == 0 ? 0 : ScaledBiasSum / (64.0 * Calls), MinimumScaledBias, MaximumScaledBias,
            new SortedDictionary<int, long>(BiasHistogram), (long[])FirstTies.Clone(),
            (long[])SecondTies.Clone(), (long[])ThirdTies.Clone());
    }

    private readonly int finalSourceFrame;
    private readonly Accumulator[] overall = { new(), new(), new() };
    private readonly Accumulator[] final = { new(), new(), new() };

    internal DcAverageDiagnostics(int finalSourceFrame)
    {
        ArgumentOutOfRangeException.ThrowIfNegative(finalSourceFrame);
        this.finalSourceFrame = finalSourceFrame;
    }

    internal void Observe(int sourceFrame, int plane, ReadOnlySpan<byte> pixels, int stride, int treeDc)
    {
        ArgumentOutOfRangeException.ThrowIfNegative(sourceFrame);
        if ((uint)plane >= 3) throw new ArgumentOutOfRangeException(nameof(plane));
        ArgumentOutOfRangeException.ThrowIfLessThan(stride, 8);
        if (pixels.Length < checked(7 * stride + 8)) throw new ArgumentException("Incomplete diagnostic block.", nameof(pixels));
        if ((uint)treeDc > 2040) throw new ArgumentOutOfRangeException(nameof(treeDc));
        int sum = 0, first = 0, second = 0, third = 0;
        for (int x = 0; x < 8; x++)
        {
            int ab = pixels[x] + pixels[x + stride];
            int cd = pixels[x + 2 * stride] + pixels[x + 3 * stride];
            int ef = pixels[x + 4 * stride] + pixels[x + 5 * stride];
            int gh = pixels[x + 6 * stride] + pixels[x + 7 * stride];
            sum += ab + cd + ef + gh;
            first += (ab & 1) + (cd & 1) + (ef & 1) + (gh & 1);
            int top = ((ab + 1) >> 1) + ((cd + 1) >> 1);
            int bottom = ((ef + 1) >> 1) + ((gh + 1) >> 1);
            second += (top & 1) + (bottom & 1);
            third += (((top + 1) >> 1) + ((bottom + 1) >> 1)) & 1;
        }
        overall[plane].Add(treeDc, sum, first, second, third);
        if (sourceFrame == finalSourceFrame) final[plane].Add(treeDc, sum, first, second, third);
    }

    internal DcAverageSnapshot Snapshot()
    {
        string[] names = { "Y", "U", "V" };
        return new(finalSourceFrame,
            "Observes the unchanged staged tree on actual refresh inputs, even when an experimental exact chroma value is used by the decoder. Call-weighted diagnostic arithmetic differences, not net frame bias or unique-block counts. DC bias=(8*treeDC-sum64)/8; pixel-equivalent bias=(8*treeDC-sum64)/64. Repeated neighbor refreshes may observe the same block more than once.",
            Enumerable.Range(0, 3).Select(i => overall[i].Snapshot(names[i])).ToArray(),
            Enumerable.Range(0, 3).Select(i => final[i].Snapshot(names[i])).ToArray());
    }
}
