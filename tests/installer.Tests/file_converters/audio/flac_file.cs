namespace InstallerTests.Audio
{
    /// <summary>The metadata blocks of a FLAC file, read straight from its bytes.</summary>
    internal sealed record FlacMetadata(IReadOnlyList<(int Type, int Length)> Blocks, int BitsPerSample, int SampleRate, int Channels)
    {
        public const int StreamInfo = 0;
        public const int VorbisComment = 4;

        public static FlacMetadata Read(string path)
        {
            byte[] file = File.ReadAllBytes(path);
            if (!file.AsSpan(0, 4).SequenceEqual("fLaC"u8)) throw new InvalidDataException($"'{path}' is not a FLAC file");
            var blocks = new List<(int, int)>();
            int position = 4;
            bool last;
            do
            {
                last = (file[position] & 0x80) != 0;
                int length = file[position + 1] << 16 | file[position + 2] << 8 | file[position + 3];
                blocks.Add((file[position] & 0x7F, length));
                position += 4 + length;
            }
            while (!last);

            ReadOnlySpan<byte> info = file.AsSpan(8, 34);
            int packed = info[10] << 12 | info[11] << 4 | info[12] >> 4;
            return new FlacMetadata(blocks, ((info[12] & 1) << 4 | info[13] >> 4) + 1, packed, ((info[12] >> 1) & 7) + 1);
        }
    }
}
