using System.Text;

namespace InstallerTests.Audio
{
    /// <summary>Synthetic RIFF WAVE files: a header, optional chunks before <c>fmt </c> and a few silent samples.</summary>
    internal static class WavFile
    {
        public static byte[] Pcm(int bitsPerSample, bool junkBeforeFormat = false, ushort formatTag = 1, bool extensible = false, int frames = 4)
        {
            using var body = new MemoryStream();
            using var writer = new BinaryWriter(body, Encoding.ASCII);
            writer.Write("WAVE"u8);
            if (junkBeforeFormat) Chunk(writer, "JUNK", new byte[27]);
            Chunk(writer, "fmt ", FormatChunk(bitsPerSample, formatTag, extensible));
            Chunk(writer, "data", Tone(bitsPerSample, frames));

            using var file = new MemoryStream();
            using var fileWriter = new BinaryWriter(file, Encoding.ASCII);
            fileWriter.Write("RIFF"u8);
            fileWriter.Write((uint)body.Length);
            fileWriter.Write(body.ToArray());
            return file.ToArray();
        }

        /// <summary>Two channels of a triangle wave, <paramref name="bitsPerSample"/> wide, so an encoder has something to code.</summary>
        private static byte[] Tone(int bitsPerSample, int frames)
        {
            int width = bitsPerSample / 8;
            var samples = new byte[width * 2 * frames];
            for (int frame = 0; frame < frames; frame++)
            {
                int level = (frame % 64 - 32) * 100;
                for (int channel = 0; channel < 2; channel++)
                    for (int part = 0; part < width; part++)
                        samples[(frame * 2 + channel) * width + part] = (byte)(level >> (part * 8));
            }
            return samples;
        }

        private static byte[] FormatChunk(int bitsPerSample, ushort formatTag, bool extensible)
        {
            using var stream = new MemoryStream();
            using var writer = new BinaryWriter(stream);
            writer.Write(extensible ? (ushort)0xFFFE : formatTag);
            writer.Write((ushort)2);
            writer.Write(48000u);
            writer.Write((uint)(48000 * 2 * bitsPerSample / 8));
            writer.Write((ushort)(2 * bitsPerSample / 8));
            writer.Write((ushort)bitsPerSample);
            if (extensible)
            {
                writer.Write((ushort)22);
                writer.Write((ushort)bitsPerSample);
                writer.Write(3u);
                writer.Write(formatTag);
                writer.Write(new byte[14]);
            }
            return stream.ToArray();
        }

        private static void Chunk(BinaryWriter writer, string id, byte[] content)
        {
            writer.Write(Encoding.ASCII.GetBytes(id));
            writer.Write((uint)content.Length);
            writer.Write(content);
            if (content.Length % 2 == 1) writer.Write((byte)0);
        }
    }
}
