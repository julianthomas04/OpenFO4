// SPDX-License-Identifier: GPL-3.0-or-later
// Container layout follows Librempeg's Bink demuxer by Peter Ross (2008-2010)
// and Daniel Verkamp (2009), licensed GPL-3.0-or-later.
using System.Buffers.Binary;

namespace OpenFallout4.Bink2;

public sealed record Bink2FramePacket(int Index, bool IsKeyFrame, byte[] Data);

public sealed record Bink2AudioTrack(int Id, int SampleRate, ushort Flags, uint MaximumDecodedBytes)
{
    public const ushort SixteenBit = 0x4000;
    public const ushort Stereo = 0x2000;
    public const ushort Dct = 0x1000;
    public const ushort KnownFlags = SixteenBit | Stereo | Dct;

    public int Channels => (Flags & Stereo) != 0 ? 2 : 1;
}

public sealed class Bink2Container : IDisposable
{
    private const int MaximumPacketBytes = 128 * 1024 * 1024;
    private const int MaximumAudioTracks = 256;
    private const uint SupportedVideoFlags = 0x10;
    private readonly FileStream stream;
    private readonly long[] offsets;
    private readonly bool[] keyFrames;
    private int nextFrame;
    public int Width { get; }
    public int Height { get; }
    public uint FrameRateNumerator { get; }
    public uint FrameRateDenominator { get; }
    public int FrameCount => keyFrames.Length;
    public IReadOnlyList<Bink2AudioTrack> AudioTracks { get; }

    private Bink2Container(FileStream stream)
    {
        this.stream = stream;
        var header = new byte[44];
        ReadExactly(header);
        if (!header.AsSpan(0, 4).SequenceEqual("KB2g"u8))
            throw new NotSupportedException("Only KB2g containers are supported.");
        uint Field(int offset) => BinaryPrimitives.ReadUInt32LittleEndian(header.AsSpan(offset));
        var logicalLength = (long)Field(4) + 8;
        var frameCount = Field(8);
        var largestPacket = Field(12);
        if (logicalLength > stream.Length || logicalLength < 44 || frameCount is 0 or > 1_000_000)
            throw new InvalidDataException("Invalid declared container length or frame count.");
        if (largestPacket == 0 || largestPacket > MaximumPacketBytes || largestPacket > logicalLength)
            throw new InvalidDataException("Invalid largest packet size.");
        Width = checked((int)Field(20));
        Height = checked((int)Field(24));
        if (Width < 64 || Height <= 0 || Width > 4096 || Height > 4096 || (Width & 31) != 0 || (Height & 1) != 0)
            throw new NotSupportedException("Only 32-aligned width from 64 through 4096 and even height up to 4096 are supported.");
        FrameRateNumerator = Field(28);
        FrameRateDenominator = Field(32);
        if (FrameRateNumerator == 0 || FrameRateDenominator == 0)
            throw new InvalidDataException("Invalid frame rate.");
        if (Field(36) != SupportedVideoFlags)
            throw new NotSupportedException($"Decoder requires video flags 0x{SupportedVideoFlags:X}.");
        var audioTrackCount = Field(40);
        if (audioTrackCount > MaximumAudioTracks)
            throw new InvalidDataException("Too many audio tracks.");
        AudioTracks = ReadAudioTracks((int)audioTrackCount);
        var tableEnd = 44L + audioTrackCount * 12L + frameCount * 4L;
        if (tableEnd > logicalLength)
            throw new InvalidDataException("Truncated frame index.");
        offsets = new long[frameCount + 1];
        keyFrames = new bool[frameCount];
        Span<byte> entry = stackalloc byte[4];
        for (var i = 0; i < frameCount; i++)
        {
            ReadExactly(entry);
            var value = BinaryPrimitives.ReadUInt32LittleEndian(entry);
            offsets[i] = value & ~1u;
            keyFrames[i] = i == 0 || (value & 1) != 0;
            if (offsets[i] < tableEnd || offsets[i] >= logicalLength)
                throw new InvalidDataException("Frame offset is outside the payload.");
        }
        offsets[frameCount] = logicalLength & ~1L;
        for (var i = 0; i < frameCount; i++)
        {
            var size = offsets[i + 1] - offsets[i];
            if (size < 4 || size > largestPacket || size > MaximumPacketBytes)
                throw new InvalidDataException("Invalid frame index ordering or packet size.");
        }
    }

    private Bink2AudioTrack[] ReadAudioTracks(int count)
    {
        var table = new byte[count * 12];
        ReadExactly(table);
        var tracks = new Bink2AudioTrack[count];
        for (var i = 0; i < count; i++)
        {
            var maximumDecodedBytes = BinaryPrimitives.ReadUInt32LittleEndian(table.AsSpan(i * 4));
            var sampleRate = BinaryPrimitives.ReadUInt16LittleEndian(table.AsSpan(count * 4 + i * 4));
            var flags = BinaryPrimitives.ReadUInt16LittleEndian(table.AsSpan(count * 4 + i * 4 + 2));
            var id = BinaryPrimitives.ReadInt32LittleEndian(table.AsSpan(count * 8 + i * 4));
            if (sampleRate == 0)
                throw new InvalidDataException("Invalid audio sample rate.");
            if ((flags & ~Bink2AudioTrack.KnownFlags) != 0)
                throw new NotSupportedException($"Unsupported audio track flags 0x{flags:X4}.");
            tracks[i] = new Bink2AudioTrack(id, sampleRate, flags, maximumDecodedBytes);
        }
        return tracks;
    }

    public static Bink2Container Open(string path)
    {
        var stream = new FileStream(path, FileMode.Open, FileAccess.Read, FileShare.Read);
        try { return new Bink2Container(stream); }
        catch { stream.Dispose(); throw; }
    }

    public Bink2FramePacket? ReadNextFrame()
    {
        if (nextFrame == FrameCount) return null;
        var index = nextFrame;
        var packet = new byte[checked((int)(offsets[index + 1] - offsets[index]))];
        stream.Position = offsets[index];
        ReadExactly(packet);
        nextFrame++;
        return new Bink2FramePacket(index, keyFrames[index], packet[VideoStart(packet, index)..]);
    }

    private int VideoStart(byte[] packet, int index)
    {
        var position = 0;
        for (var track = 0; track < AudioTracks.Count; track++)
        {
            if (packet.Length - position < 4)
                throw new InvalidDataException($"Frame {index}: truncated audio packet size for track {track}.");
            var audioBytes = BinaryPrimitives.ReadUInt32LittleEndian(packet.AsSpan(position));
            position += 4;
            if (audioBytes > packet.Length - position)
                throw new InvalidDataException($"Frame {index}: audio packet of track {track} exceeds the frame.");
            position += (int)audioBytes;
        }
        return position;
    }

    private void ReadExactly(Span<byte> destination)
    {
        try { stream.ReadExactly(destination); }
        catch (EndOfStreamException error) { throw new InvalidDataException("Truncated container.", error); }
    }

    public void Dispose() => stream.Dispose();
}
