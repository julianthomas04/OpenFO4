using System.Buffers.Binary;
using System.Text;
using AssetConverter;

namespace InstallerTests
{
    /// <summary>One water quad of a synthetic block: offset from the block origin in cells and its height in game units.</summary>
    internal readonly record struct FixtureQuad(int CellOffsetX, int CellOffsetY, double Height);

    /// <summary>
    /// Builds a minimal terrain LOD file (NIF 20.2.0.7, BS 130): a root BSMultiBoundNode, a WATER BSMultiBoundNode and one
    /// BSSubIndexTriShape per quad list, in the layout BtrWaterReader documents.
    /// </summary>
    internal sealed class BtrBuilder
    {
        readonly List<List<FixtureQuad>> _shapes = [];

        public float ShapeScale { get; set; } = 4f;
        public ulong VertexDescriptor { get; set; } = BtrWaterReader.WaterVertexDescriptor;
        public float WaterNodeScale { get; set; } = 1f;
        public string ShapeType { get; set; } = "BSSubIndexTriShape";
        public bool IncludeWaterNode { get; set; } = true;
        public int TriangleIndexShift { get; set; }
        public double VertexZSkew { get; set; }

        public BtrBuilder Shape(params FixtureQuad[] quads)
        {
            _shapes.Add([.. quads]);
            return this;
        }

        public byte[] Build()
        {
            var strings = new List<string> { "chunk", "Land", "WATER" };
            var blocks = new List<(string Type, byte[] Data)>
            {
                ("BSMultiBoundNode", Node(0, IncludeWaterNode ? [1] : [], 1f)),
            };
            if (IncludeWaterNode)
                blocks.Add(("BSMultiBoundNode", Node(2, Enumerable.Range(2, _shapes.Count).ToArray(), WaterNodeScale)));
            foreach (var quads in _shapes) blocks.Add((ShapeType, Shape(quads)));
            return Header(blocks, strings);
        }

        static byte[] Node(int nameIndex, int[] children, float scale)
        {
            var w = new BinaryWriter(new MemoryStream());
            AvObject(w, nameIndex, scale);
            w.Write((uint)children.Length);
            foreach (int child in children) w.Write(child);
            w.Write(-1);
            w.Write(0u);
            return ((MemoryStream)w.BaseStream).ToArray();
        }

        static void AvObject(BinaryWriter w, int nameIndex, float scale)
        {
            w.Write(nameIndex);
            w.Write(0u);
            w.Write(-1);
            w.Write(0u);
            for (int i = 0; i < 3; i++) w.Write(0f);
            foreach (float r in new float[] { 1, 0, 0, 0, 1, 0, 0, 0, 1 }) w.Write(r);
            w.Write(scale);
            w.Write(-1);
        }

        byte[] Shape(List<FixtureQuad> quads)
        {
            var w = new BinaryWriter(new MemoryStream());
            AvObject(w, -1, ShapeScale);
            for (int i = 0; i < 4; i++) w.Write(0f);
            w.Write(-1);
            w.Write(-1);
            w.Write(-1);
            w.Write(VertexDescriptor);
            w.Write((uint)(quads.Count * 2));
            w.Write((ushort)(quads.Count * 4));

            var vertices = new MemoryStream();
            var triangles = new MemoryStream();
            for (int q = 0; q < quads.Count; q++)
            {
                double x0 = quads[q].CellOffsetX * BtrWaterReader.CellSize, y0 = quads[q].CellOffsetY * BtrWaterReader.CellSize;
                foreach (var (dx, dy) in new[] { (0, 0), (1, 0), (0, 1), (1, 1) })
                {
                    double[] components =
                    [
                        x0 + dx * BtrWaterReader.CellSize,
                        y0 + dy * BtrWaterReader.CellSize,
                        quads[q].Height + (dx + dy == 2 ? VertexZSkew : 0),
                        0,
                    ];
                    foreach (double value in components) WriteHalf(vertices, value / ShapeScale);
                }
                foreach (int index in new[] { 0, 1, 2, 1, 3, 2 })
                {
                    var bytes = new byte[2];
                    BinaryPrimitives.WriteUInt16LittleEndian(bytes, (ushort)(q * 4 + index + (q == 0 ? TriangleIndexShift : 0)));
                    triangles.Write(bytes);
                }
            }
            w.Write((uint)(vertices.Length + triangles.Length));
            w.Write(vertices.ToArray());
            w.Write(triangles.ToArray());
            return ((MemoryStream)w.BaseStream).ToArray();
        }

        static void WriteHalf(Stream stream, double value)
        {
            var bytes = new byte[2];
            BinaryPrimitives.WriteHalfLittleEndian(bytes, (Half)value);
            stream.Write(bytes);
        }

        static byte[] Header(List<(string Type, byte[] Data)> blocks, List<string> strings)
        {
            var types = blocks.Select(b => b.Type).Distinct().ToList();
            var w = new BinaryWriter(new MemoryStream());
            w.Write(Encoding.ASCII.GetBytes("Gamebryo File Format, Version 20.2.0.7\n"));
            w.Write(0x14020007u);
            w.Write((byte)1);
            w.Write(12u);
            w.Write((uint)blocks.Count);
            w.Write(130u);
            for (int i = 0; i < 4; i++) w.Write((byte)0);
            w.Write((ushort)types.Count);
            foreach (var type in types) { w.Write((uint)type.Length); w.Write(Encoding.ASCII.GetBytes(type)); }
            foreach (var block in blocks) w.Write((ushort)types.IndexOf(block.Type));
            foreach (var block in blocks) w.Write((uint)block.Data.Length);
            w.Write((uint)strings.Count);
            w.Write((uint)strings.Max(s => s.Length));
            foreach (var s in strings) { w.Write((uint)s.Length); w.Write(Encoding.ASCII.GetBytes(s)); }
            w.Write(0u);
            foreach (var block in blocks) w.Write(block.Data);
            return ((MemoryStream)w.BaseStream).ToArray();
        }
    }
}
