// Modified for OpenFO4 in 2026, including provenance notices; GPL-3.0-only. See SOURCE.md.
using System.IO;

namespace SharpBSABA2.BSAUtil
{
    public struct BSAHeaderMW
    {
        public const uint Size = 12;

        public uint Version { get; private set; }
        public uint HashOffset { get; private set; }
        public uint FileCount { get; private set; }

        public BSAHeaderMW(BinaryReader br, uint version)
        {
            Version = version;
            HashOffset = br.ReadUInt32();
            FileCount = br.ReadUInt32();
        }
    }
}
