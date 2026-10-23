// Modified for OpenFO4 in 2026, including provenance notices; GPL-3.0-only. See SOURCE.md.
using System.IO;

namespace SharpBSABA2.BA2Util
{
    public struct BA2TextureChunk
    {
        public ulong offset { get; set; }
        public uint packSz { get; set; }
        public uint fullSz { get; set; }
        public ushort startMip { get; set; }
        public ushort endMip { get; set; }
        public uint align { get; set; }

        public BA2TextureChunk(BinaryReader br)
        {
            offset = br.ReadUInt64();
            packSz = br.ReadUInt32();
            fullSz = br.ReadUInt32();
            startMip = br.ReadUInt16();
            endMip = br.ReadUInt16();
            align = br.ReadUInt32();
        }
    }
}
