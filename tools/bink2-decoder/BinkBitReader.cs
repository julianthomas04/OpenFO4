// SPDX-License-Identifier: GPL-3.0-or-later
namespace OpenFallout4.Bink2;

internal sealed class BinkBitReader
{
    private readonly byte[] data;
    public int Position { get; private set; }
    public int BitsLeft => data.Length * 8 - Position;

    public BinkBitReader(byte[] data)
    {
        ArgumentNullException.ThrowIfNull(data);
        if (data.Length > int.MaxValue / 8)
            throw new ArgumentOutOfRangeException(nameof(data));
        this.data = data;
    }

    public uint ReadBits(int count)
    {
        if (count is < 0 or > 32)
            throw new ArgumentOutOfRangeException(nameof(count));
        EnsureAvailable(count);
        uint value = 0;
        for (var bit = 0; bit < count; bit++)
            value |= (uint)((data[(Position + bit) >> 3] >> ((Position + bit) & 7)) & 1) << bit;
        Position += count;
        return value;
    }

    public int ReadBit() => (int)ReadBits(1);

    public int ReadUnary(int stopBit, int max)
    {
        if (stopBit is < 0 or > 1) throw new ArgumentOutOfRangeException(nameof(stopBit));
        ArgumentOutOfRangeException.ThrowIfNegative(max);
        var count = 0;
        while (count < max && ReadBit() != stopBit)
            count++;
        return count;
    }

    public void Skip(int count)
    {
        ArgumentOutOfRangeException.ThrowIfNegative(count);
        EnsureAvailable(count);
        Position += count;
    }

    public void AlignByte() => Skip((-Position) & 7);

    private void EnsureAvailable(int count)
    {
        if (count > BitsLeft)
            throw new InvalidDataException($"Truncated bitstream at bit {Position}: requested {count}, available {BitsLeft}.");
    }
}
