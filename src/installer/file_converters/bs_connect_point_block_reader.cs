using System.Numerics;
using System.Text;
using NiflySharp;
using NiflySharp.Blocks;

namespace AssetConverter
{
    // NiflySharp does not map the "BSConnectPoint::Parents" / "BSConnectPoint::Children" block type names
    // to its BSConnectPoint_* classes, so both arrive as NiUnknown with their raw payload. Layouts follow
    // nif.xml (FO4): the NiExtraData name (string table index), then the block fields; strings are
    // uint32-length sized, quaternions are stored w, x, y, z.
    internal static class BsConnectPointBlockReader
    {
        public const string ParentsTypeName = "BSConnectPoint::Parents";
        public const string ChildrenTypeName = "BSConnectPoint::Children";

        internal sealed record ParentPoint(string Parent, string Name, Quaternion Rotation, Vector3 Translation, float Scale);

        internal sealed record ChildPoints(bool Skinned, IReadOnlyList<string> Names);

        public static IEnumerable<int> BlocksOfType(NifFile nif, string typeName) =>
            Enumerable.Range(0, nif.Blocks.Count).Where(block => nif.Header.GetBlockTypeNameById(block) == typeName);

        public static IReadOnlyList<ParentPoint> ReadParents(NifFile nif, int block)
        {
            var reader = Open(nif, block, ParentsTypeName);
            reader.ReadInt32();
            uint count = reader.ReadUInt32();
            var points = new List<ParentPoint>();
            for (uint i = 0; i < count; i++)
            {
                string parent = ReadSizedString(reader);
                string name = ReadSizedString(reader);
                float w = reader.ReadSingle(), x = reader.ReadSingle(), y = reader.ReadSingle(), z = reader.ReadSingle();
                var translation = new Vector3(reader.ReadSingle(), reader.ReadSingle(), reader.ReadSingle());
                points.Add(new ParentPoint(parent, name, new Quaternion(x, y, z, w), translation, reader.ReadSingle()));
            }
            return points;
        }

        public static ChildPoints ReadChildren(NifFile nif, int block)
        {
            var reader = Open(nif, block, ChildrenTypeName);
            reader.ReadInt32();
            bool skinned = reader.ReadByte() != 0;
            uint count = reader.ReadUInt32();
            var names = new List<string>();
            for (uint i = 0; i < count; i++)
                names.Add(ReadSizedString(reader));
            return new ChildPoints(skinned, names);
        }

        private static string ReadSizedString(BinaryReader reader)
        {
            uint length = reader.ReadUInt32();
            long remaining = reader.BaseStream.Length - reader.BaseStream.Position;
            if (length > remaining)
                throw new NifConnectPointException($"string of {length} bytes but only {remaining} bytes remain");
            return Encoding.Latin1.GetString(reader.ReadBytes((int)length)).TrimEnd('\0');
        }

        private static BinaryReader Open(NifFile nif, int block, string expectedTypeName)
        {
            string actual = nif.Header.GetBlockTypeNameById(block);
            if (actual != expectedTypeName)
                throw new NifConnectPointException($"block {block} is {actual}, expected {expectedTypeName}");
            if (nif.Blocks[block] is not NiUnknown raw || raw.Data == null)
                throw new NifConnectPointException($"block {block} ({expectedTypeName}) was parsed as {nif.Blocks[block]?.GetType().Name} and has no raw payload");
            return new BinaryReader(new MemoryStream(raw.Data, writable: false));
        }
    }
}
