using System.Numerics;
using System.Text;

namespace InstallerTests
{
    /// <summary>
    /// A hand-assembled FO4 NIF whose root NiNode carries BSConnectPoint::Parents and ::Children extra data
    /// (NiflySharp reads those two block types as raw NiUnknown payloads), with one child NiNode "Receiver".
    /// Quaternions are written in file order w, x, y, z.
    /// </summary>
    internal sealed class ConnectPointNif
    {
        public sealed record ParentPoint(string Parent, string Name, Quaternion Rotation, Vector3 Translation, float Scale = 1f);

        public List<ParentPoint> Parents { get; } = [];

        public List<string>? ChildNames { get; set; }

        public bool Skinned { get; set; }

        public Vector3 RootTranslation { get; set; }

        public float RootScale { get; set; } = 1f;

        public bool RootHidden { get; set; }

        public bool TruncateParentsPayload { get; set; }

        public bool ReceiverIsBillboard { get; set; }

        const uint VisibleFlags = 14u, HiddenFlags = VisibleFlags | 1u;

        public byte[] ToBytes()
        {
            const int RootNameIndex = 0, ReceiverNameIndex = 1, ParentsNameIndex = 2, ChildrenNameIndex = 3;
            string[] strings = ["Root", "Receiver", "BSConnectPoint::Parents", "BSConnectPoint::Children"];
            var extraBlocks = new List<(string Type, byte[] Payload)>();
            if (Parents.Count > 0)
                extraBlocks.Add(("BSConnectPoint::Parents", ParentsBlock(ParentsNameIndex)));
            if (ChildNames != null)
                extraBlocks.Add(("BSConnectPoint::Children", ChildrenBlock(ChildrenNameIndex)));

            int receiverBlock = 1 + extraBlocks.Count;
            var extraBlockIndices = Enumerable.Range(1, extraBlocks.Count).ToList();
            var blocks = new List<(string Type, byte[] Payload)> { ("NiNode", NiNodeBlock(RootNameIndex, extraBlockIndices, RootTranslation, RootScale, [receiverBlock], RootHidden ? HiddenFlags : VisibleFlags)) };
            blocks.AddRange(extraBlocks);
            blocks.Add(ReceiverIsBillboard
                ? ("NiBillboardNode", NiNodeBlock(ReceiverNameIndex, [], Vector3.Zero, 1f, [], VisibleFlags).Concat(new byte[sizeof(ushort)]).ToArray())
                : ("NiNode", NiNodeBlock(ReceiverNameIndex, [], Vector3.Zero, 1f, [], VisibleFlags)));
            return Assemble(blocks, strings);
        }

        byte[] ParentsBlock(int nameIndex)
        {
            using var stream = new MemoryStream();
            using var writer = new BinaryWriter(stream);
            writer.Write(nameIndex);
            writer.Write((uint)Parents.Count);
            foreach (var point in Parents)
            {
                WriteSized(writer, point.Parent);
                WriteSized(writer, point.Name);
                writer.Write(point.Rotation.W);
                writer.Write(point.Rotation.X);
                writer.Write(point.Rotation.Y);
                writer.Write(point.Rotation.Z);
                writer.Write(point.Translation.X);
                writer.Write(point.Translation.Y);
                writer.Write(point.Translation.Z);
                writer.Write(point.Scale);
            }
            byte[] payload = stream.ToArray();
            return TruncateParentsPayload ? payload[..^8] : payload;
        }

        byte[] ChildrenBlock(int nameIndex)
        {
            using var stream = new MemoryStream();
            using var writer = new BinaryWriter(stream);
            writer.Write(nameIndex);
            writer.Write((byte)(Skinned ? 1 : 0));
            writer.Write((uint)ChildNames!.Count);
            foreach (var name in ChildNames)
                WriteSized(writer, name);
            return stream.ToArray();
        }

        static byte[] NiNodeBlock(int nameIndex, IReadOnlyList<int> extraData, Vector3 translation, float scale, IReadOnlyList<int> children, uint flags)
        {
            using var stream = new MemoryStream();
            using var writer = new BinaryWriter(stream);
            writer.Write(nameIndex);
            writer.Write((uint)extraData.Count);
            foreach (int block in extraData) writer.Write(block);
            writer.Write(-1);
            writer.Write(flags);
            writer.Write(translation.X);
            writer.Write(translation.Y);
            writer.Write(translation.Z);
            foreach (float m in new float[] { 1, 0, 0, 0, 1, 0, 0, 0, 1 }) writer.Write(m);
            writer.Write(scale);
            writer.Write(-1);
            writer.Write((uint)children.Count);
            foreach (int child in children) writer.Write(child);
            return stream.ToArray();
        }

        static void WriteSized(BinaryWriter writer, string value)
        {
            writer.Write((uint)value.Length);
            writer.Write(Encoding.Latin1.GetBytes(value));
        }

        static byte[] Assemble(IReadOnlyList<(string Type, byte[] Payload)> blocks, string[] strings)
        {
            var types = blocks.Select(b => b.Type).Distinct().ToList();
            using var stream = new MemoryStream();
            using var writer = new BinaryWriter(stream);
            writer.Write(Encoding.ASCII.GetBytes("Gamebryo File Format, Version 20.2.0.7\n"));
            writer.Write(0x14020007u);
            writer.Write((byte)1);
            writer.Write(12u);
            writer.Write((uint)blocks.Count);
            writer.Write(130u);
            for (int i = 0; i < 4; i++) writer.Write((byte)0);
            writer.Write((ushort)types.Count);
            foreach (var type in types) WriteSized(writer, type);
            foreach (var block in blocks) writer.Write((ushort)types.IndexOf(block.Type));
            foreach (var block in blocks) writer.Write((uint)block.Payload.Length);
            writer.Write((uint)strings.Length);
            writer.Write((uint)strings.Max(s => s.Length));
            foreach (var value in strings) WriteSized(writer, value);
            writer.Write(0u);
            foreach (var block in blocks) writer.Write(block.Payload);
            writer.Write(1u);
            writer.Write(0u);
            return stream.ToArray();
        }
    }
}
