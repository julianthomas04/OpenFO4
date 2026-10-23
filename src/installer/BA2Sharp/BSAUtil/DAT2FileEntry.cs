// Modified for OpenFO4 in 2026, including provenance notices; GPL-3.0-only. See SOURCE.md.
using System.IO;
using SharpBSABA2.Extensions;

namespace SharpBSABA2.BSAUtil
{
    public struct DAT2FileEntry
    {
        public int FilenameSize { get; set; }
        public string Filename { get; set; }
        public bool Compressed { get; set; }
        public uint RealSize { get; set; }
        public uint PackedSize { get; set; }
        public uint Offset { get; set; }

        public DAT2FileEntry(BinaryReader reader)
        {
            this.FilenameSize = reader.ReadInt32();
            this.Filename = reader.ReadString(this.FilenameSize).TrimStart('\\');
            this.Compressed = reader.ReadByte() == 1;
            this.RealSize = reader.ReadUInt32();
            this.PackedSize = reader.ReadUInt32();
            this.Offset = reader.ReadUInt32();
        }
    }
}
