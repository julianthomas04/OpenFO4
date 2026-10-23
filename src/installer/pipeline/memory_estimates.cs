namespace AssetConverter.Pipeline
{
    /// <summary>
    /// What each kind of work unit reserves from the <see cref="MemoryBudget"/>, in one place so the figures can be checked against
    /// measurements together. Every figure is the commit the unit was measured to need, with the input it holds in memory added where
    /// that scales with the input.
    /// </summary>
    internal static class MemoryEstimates
    {
        private const long MiB = 1L << 20;

        /// <summary>The ESM import: Mutagen overlay, the raw scans and the SQLite connections, for the whole load.</summary>
        public const long EsmImportBytes = 2800 * MiB;

        /// <summary>One video conversion: the Bink2 decoder and the AV1 encoder.</summary>
        public const long VideoConversionBytes = 1600 * MiB;

        /// <summary>One ffdec process: a JVM started with <c>-Xmx1024m</c> and its native overhead.</summary>
        public const long FfdecProcessBytes = 1250 * MiB;

        /// <summary>One audio conversion besides the entry's content: the encoder process.</summary>
        public const long AudioEncoderProcessBytes = 64 * MiB;

        /// <summary>One collision pool process: an average of about 100 MB, up to about 340 MB.</summary>
        public const long CollisionPoolProcessBytes = 350 * MiB;

        /// <summary>The largest share of the budget the collision pool may hold for the whole install, which keeps the extraction it waits on able to run beside it.</summary>
        public const double CollisionPoolCapacityShare = 0.25;

        private const long NifConversionBaseBytes = 48 * MiB;
        private const long NifConversionBytesPerInputByte = 8;
        private const long HavokClipPackBaseBytes = 64 * MiB;
        private const long HavokClipPackBytesPerInputByte = 64;
        private const long PipboyArtClipBaseBytes = 32 * MiB;
        private const long PipboySymbolClipBytesPerFrameFileByte = 4;
        private const long PipboyMovieClipBytesPerFrameFileByte = 2;

        public static long AudioConversionBytes(long contentLength) => AudioEncoderProcessBytes + contentLength;

        public static long NifConversionBytes(long fileLength) => NifConversionBaseBytes + NifConversionBytesPerInputByte * fileLength;

        /// <summary>
        /// One Havok clip pack from decode to save: the decoded clips stay in the glTF model until it is written. Measured managed
        /// heap peaks reach about 62 times the pack's input for the largest packs, so the biggest ones run alone.
        /// </summary>
        public static long HavokClipPackBytes(long totalClipLength) => HavokClipPackBaseBytes + HavokClipPackBytesPerInputByte * totalClipLength;

        /// <summary>
        /// One Pip-Boy symbol clip from reading its ffdec frames to writing them, given the length of its frame files counted once per
        /// frame of the clip. The ASCII files become UTF-16 strings, two bytes per file byte, and the rebased copy of every frame is made
        /// while the read frames are still held, which gives two copies at the peak. The base covers the transient copies of the
        /// frame in hand (the read buffer, the rebased string, its UTF-8 bytes and gzip output, together about ten times one frame, and
        /// the largest exported frames are about 250 KB) with a wide margin.
        /// </summary>
        public static long PipboySymbolClipBytes(long frameFileLength) =>
            PipboyArtClipBaseBytes + PipboySymbolClipBytesPerFrameFileByte * frameFileLength;

        /// <summary>
        /// One Pip-Boy movie clip from reading its ffdec frames to writing them: frames are read and rebased one at a time, so only the
        /// rebased copies, UTF-16 strings of two bytes per byte of the ASCII files, are held at the peak. The base is as for
        /// <see cref="PipboySymbolClipBytes"/>.
        /// </summary>
        public static long PipboyMovieClipBytes(long frameFileLength) =>
            PipboyArtClipBaseBytes + PipboyMovieClipBytesPerFrameFileByte * frameFileLength;
    }
}
