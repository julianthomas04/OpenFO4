namespace InstallerTests
{
    /// <summary>
    /// Uncompressed DDS files built from the Microsoft DDS programming guide layout: "DDS " magic, 124-byte DDS_HEADER
    /// (size, flags, height, width, pitch, depth, mip count, 11 reserved, 32-byte DDS_PIXELFORMAT, caps) and the pixel data.
    /// </summary>
    internal static class SyntheticDds
    {
        public const int HeaderSize = 128;

        const uint Magic = 0x20534444;
        const uint CapsFlags = 0x1 | 0x2 | 0x4 | 0x1000;
        const uint PixelFormatRgb = 0x40;
        const uint PixelFormatAlphaPixels = 0x1;
        const uint CapsTexture = 0x1000;

        /// <summary>32-bit BGRA; <paramref name="bgra"/> holds width*height*4 bytes, row-major, blue first.</summary>
        public static byte[] Bgra32(int width, int height, byte[] bgra) =>
            Build(width, height, 32, PixelFormatRgb | PixelFormatAlphaPixels, 0x00FF0000, 0x0000FF00, 0x000000FF, 0xFF000000, bgra);

        /// <summary>24-bit BGR; <paramref name="bgr"/> holds width*height*3 bytes (rows are 4-byte aligned only when the width allows it).</summary>
        public static byte[] Bgr24(int width, int height, byte[] bgr) =>
            Build(width, height, 24, PixelFormatRgb, 0x00FF0000, 0x0000FF00, 0x000000FF, 0, bgr);

        static byte[] Build(int width, int height, int bitCount, uint pixelFormatFlags, uint rMask, uint gMask, uint bMask, uint aMask, byte[] pixels)
        {
            using var stream = new MemoryStream();
            using var writer = new BinaryWriter(stream);
            writer.Write(Magic);
            writer.Write(124u);
            writer.Write(CapsFlags);
            writer.Write((uint)height);
            writer.Write((uint)width);
            writer.Write((uint)(width * bitCount / 8));
            writer.Write(0u);
            writer.Write(1u);
            writer.Write(new byte[11 * 4]);
            writer.Write(32u);
            writer.Write(pixelFormatFlags);
            writer.Write(0u);
            writer.Write((uint)bitCount);
            writer.Write(rMask);
            writer.Write(gMask);
            writer.Write(bMask);
            writer.Write(aMask);
            writer.Write(CapsTexture);
            writer.Write(new byte[4 * 4]);
            writer.Write(pixels);
            writer.Flush();
            return stream.ToArray();
        }
    }
}
