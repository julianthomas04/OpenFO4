using System.Buffers.Binary;
using System.Globalization;
using System.Text;

namespace AssetConverter
{
    /// <summary>A terrain LOD file does not have the exact structure the LOD water reader supports.</summary>
    public sealed class BtrFormatException : FormatException
    {
        public BtrFormatException(string entry, string message)
            : base($"{entry}: {message}") { }
    }

    /// <summary>One distant-LOD water quad: the cell it covers and its baked height in game units.</summary>
    public readonly record struct LodWaterQuad(int CellX, int CellY, double Height);

    /// <summary>
    /// Reads the water shapes of one terrain LOD block (<c>.BTR</c>, a NIF file, version 20.2.0.7 / BS stream 130).
    /// The water lives under a <c>BSMultiBoundNode</c> named <c>WATER</c> whose children are
    /// <c>BSSubIndexTriShape</c>s with vertex descriptor <see cref="WaterVertexDescriptor"/> (three half floats plus an
    /// unused half, 8 bytes). Every quad is four vertices (a 4096 x 4096 unit square, its x/y scaled by the shape
    /// scale) indexed as the triangles (0,1,2) and (1,3,2); the quad height is the shared vertex z. Anything else
    /// throws <see cref="BtrFormatException"/>.
    /// </summary>
    public static class BtrWaterReader
    {
        public const ulong WaterVertexDescriptor = 0x100000000002;
        public const int CellSize = 4096;
        const string WaterNodeType = "BSMultiBoundNode";
        const string WaterNodeName = "WATER";
        const string WaterShapeType = "BSSubIndexTriShape";
        const int VertexSize = 8;
        const int VerticesPerQuad = 4;
        const int TrianglesPerQuad = 2;

        /// <summary>The water quads of a block whose lowest cell is (<paramref name="blockX"/>, <paramref name="blockY"/>); empty when the block has no WATER node.</summary>
        public static IReadOnlyList<LodWaterQuad> Read(byte[] btr, string entry, int blockX, int blockY)
        {
            var nif = NifLayout.Parse(btr, entry);
            var waterNodes = Enumerable.Range(0, nif.Blocks.Count)
                .Where(i => nif.Blocks[i].Type == WaterNodeType && nif.NodeName(i) == WaterNodeName)
                .ToList();
            if (waterNodes.Count == 0) return [];
            if (waterNodes.Count > 1)
                throw new BtrFormatException(entry, $"{waterNodes.Count} nodes named {WaterNodeName}, expected one");

            RequireIdentityNode(nif, 0, entry);
            RequireIdentityNode(nif, waterNodes[0], entry);

            var quads = new List<LodWaterQuad>();
            foreach (int child in nif.NodeChildren(waterNodes[0]))
            {
                if (child < 0 || child >= nif.Blocks.Count)
                    throw new BtrFormatException(entry, $"{WaterNodeName} child reference {child} is out of range");
                if (nif.Blocks[child].Type != WaterShapeType)
                    throw new BtrFormatException(entry, $"{WaterNodeName} child {child} is a {nif.Blocks[child].Type}, expected {WaterShapeType}");
                ReadShape(nif, child, entry, blockX, blockY, quads);
            }
            return quads;
        }

        static void RequireIdentityNode(NifLayout nif, int block, string entry)
        {
            if (!nif.ReadAvObject(block).IsIdentity)
                throw new BtrFormatException(entry, $"block {block} ({nif.Blocks[block].Type}) has a non-identity transform; quad coordinates would not be block-local");
        }

        static void ReadShape(NifLayout nif, int block, string entry, int blockX, int blockY, List<LodWaterQuad> quads)
        {
            var cursor = nif.CursorAt(block);
            var transform = nif.ReadAvObject(cursor);
            if (transform.Translation != (0f, 0f, 0f) || !transform.RotationIsIdentity || !(transform.Scale > 0))
                throw new BtrFormatException(entry, $"water shape {block} has a translation, rotation or non-positive scale that the reader does not model");

            cursor.Skip(4 * 4 + 3 * 4);
            ulong descriptor = cursor.U64();
            uint triangleCount = cursor.U32();
            ushort vertexCount = cursor.U16();
            uint dataSize = cursor.U32();
            if (descriptor != WaterVertexDescriptor)
                throw new BtrFormatException(entry, $"water shape {block} has vertex descriptor 0x{descriptor:X}, expected 0x{WaterVertexDescriptor:X}");
            if (dataSize == 0 || vertexCount == 0 || vertexCount % VerticesPerQuad != 0
                || triangleCount != (uint)(vertexCount / VerticesPerQuad * TrianglesPerQuad))
                throw new BtrFormatException(entry, $"water shape {block} has {vertexCount} vertices / {triangleCount} triangles, expected 4 vertices and 2 triangles per quad");

            var vertices = cursor.Slice(vertexCount * VertexSize);
            var triangles = cursor.Slice((int)triangleCount * 6);

            int quadCount = vertexCount / VerticesPerQuad;
            for (int q = 0; q < quadCount; q++)
            {
                RequireQuadTopology(triangles, q, entry, block);
                quads.Add(ReadQuad(vertices, q, transform.Scale, entry, block, blockX, blockY));
            }
        }

        static void RequireQuadTopology(ReadOnlySpan<byte> triangles, int quad, string entry, int block)
        {
            ReadOnlySpan<int> expected = [0, 1, 2, 1, 3, 2];
            for (int i = 0; i < expected.Length; i++)
            {
                int index = BinaryPrimitives.ReadUInt16LittleEndian(triangles[((quad * 6 + i) * 2)..]);
                if (index != quad * VerticesPerQuad + expected[i])
                    throw new BtrFormatException(entry, $"water shape {block} quad {quad} is not the (0,1,2),(1,3,2) layout");
            }
        }

        static LodWaterQuad ReadQuad(ReadOnlySpan<byte> vertices, int quad, float scale, string entry, int block, int blockX, int blockY)
        {
            var x = new double[VerticesPerQuad];
            var y = new double[VerticesPerQuad];
            var z = new double[VerticesPerQuad];
            for (int v = 0; v < VerticesPerQuad; v++)
            {
                var vertex = vertices[((quad * VerticesPerQuad + v) * VertexSize)..];
                x[v] = (double)BinaryPrimitives.ReadHalfLittleEndian(vertex) * scale;
                y[v] = (double)BinaryPrimitives.ReadHalfLittleEndian(vertex[2..]) * scale;
                z[v] = (double)BinaryPrimitives.ReadHalfLittleEndian(vertex[4..]) * scale;
            }

            double minX = x.Min(), minY = y.Min();
            bool corners = x[0] == minX && y[0] == minY && x[1] == minX + CellSize && y[1] == minY
                           && x[2] == minX && y[2] == minY + CellSize && x[3] == minX + CellSize && y[3] == minY + CellSize;
            if (!corners || minX % CellSize != 0 || minY % CellSize != 0)
                throw new BtrFormatException(entry, $"water shape {block} quad {quad} is not a {CellSize} unit square on the cell grid");
            if (z.Any(h => h != z[0]) || !double.IsFinite(z[0]))
                throw new BtrFormatException(entry, $"water shape {block} quad {quad} is not flat (z {string.Join(", ", z.Select(h => h.ToString(CultureInfo.InvariantCulture)))})");

            return new LodWaterQuad(blockX + (int)(minX / CellSize), blockY + (int)(minY / CellSize), z[0]);
        }
    }

    /// <summary>The node-level part of an NIF 20.2.0.7 / BS 130 file: block table, string table and just enough of NiNode / NiAVObject to walk it.</summary>
    internal sealed class NifLayout
    {
        public readonly record struct Block(string Type, int Offset, int Size);

        public readonly record struct AvObject(string? Name, (float, float, float) Translation, float[] Rotation, float Scale)
        {
            static readonly float[] Identity = [1, 0, 0, 0, 1, 0, 0, 0, 1];
            public bool RotationIsIdentity => Rotation.AsSpan().SequenceEqual(Identity);
            public bool IsIdentity => Translation == (0f, 0f, 0f) && RotationIsIdentity && Scale == 1f;
        }

        const string FileMagic = "Gamebryo File Format, Version 20.2.0.7";
        const uint FileVersion = 0x14020007;
        const uint FalloutBsVersion = 130;

        readonly byte[] _data;
        readonly string _entry;
        readonly List<string> _strings;

        NifLayout(byte[] data, string entry, List<Block> blocks, List<string> strings)
        {
            _data = data;
            _entry = entry;
            Blocks = blocks;
            _strings = strings;
        }

        public IReadOnlyList<Block> Blocks { get; }

        public static NifLayout Parse(byte[] data, string entry)
        {
            int newline = Array.IndexOf(data, (byte)'\n');
            if (newline < 0 || Encoding.ASCII.GetString(data, 0, newline) != FileMagic)
                throw new BtrFormatException(entry, "not a Gamebryo 20.2.0.7 file");
            var c = new NifCursor(data, newline + 1, entry);
            if (c.U32() != FileVersion) throw new BtrFormatException(entry, "unexpected NIF version");
            if (c.U8() != 1) throw new BtrFormatException(entry, "big-endian NIF");
            c.U32();
            uint blockCount = c.U32();
            uint bsVersion = c.U32();
            if (bsVersion != FalloutBsVersion) throw new BtrFormatException(entry, $"BS stream version {bsVersion}, expected {FalloutBsVersion}");
            for (int i = 0; i < 4; i++) c.Skip(c.U8());

            int typeCount = c.U16();
            var types = new string[typeCount];
            for (int i = 0; i < typeCount; i++) types[i] = c.SizedString();
            if (blockCount > data.Length) throw new BtrFormatException(entry, $"implausible block count {blockCount}");
            var typeIndex = new int[blockCount];
            for (int i = 0; i < blockCount; i++) typeIndex[i] = c.U16();
            var sizes = new int[blockCount];
            for (int i = 0; i < blockCount; i++) sizes[i] = checked((int)c.U32());
            uint stringCount = c.U32();
            c.U32();
            var strings = new List<string>();
            for (uint i = 0; i < stringCount; i++) strings.Add(c.SizedString());
            uint groupCount = c.U32();
            c.Skip(checked((int)groupCount * 4));

            var blocks = new List<Block>((int)blockCount);
            long offset = c.Position;
            for (int i = 0; i < blockCount; i++)
            {
                if (typeIndex[i] >= typeCount) throw new BtrFormatException(entry, $"block {i} has type index {typeIndex[i]} out of range");
                blocks.Add(new Block(types[typeIndex[i]], (int)offset, sizes[i]));
                offset += sizes[i];
            }
            if (offset > data.Length) throw new BtrFormatException(entry, "block table extends past the end of the file");
            return new NifLayout(data, entry, blocks, strings);
        }

        public NifCursor CursorAt(int block) => new(_data, Blocks[block].Offset, _entry);

        public AvObject ReadAvObject(int block) => ReadAvObject(CursorAt(block));

        public AvObject ReadAvObject(NifCursor c)
        {
            int nameIndex = c.I32();
            uint extraDataCount = c.U32();
            c.Skip(checked((int)extraDataCount * 4));
            c.I32();
            c.U32();
            var translation = (c.F32(), c.F32(), c.F32());
            var rotation = new float[9];
            for (int i = 0; i < rotation.Length; i++) rotation[i] = c.F32();
            float scale = c.F32();
            c.I32();
            string? name = nameIndex < 0 ? null : nameIndex < _strings.Count ? _strings[nameIndex]
                : throw new BtrFormatException(_entry, $"string index {nameIndex} out of range");
            return new AvObject(name, translation, rotation, scale);
        }

        public string? NodeName(int block) => ReadAvObject(block).Name;

        public IReadOnlyList<int> NodeChildren(int block)
        {
            var c = CursorAt(block);
            ReadAvObject(c);
            uint count = c.U32();
            c.Require(checked((int)count * 4));
            var children = new int[count];
            for (int i = 0; i < count; i++) children[i] = c.I32();
            return children;
        }
    }

    /// <summary>Little-endian forward reader over a NIF byte array that throws <see cref="BtrFormatException"/> instead of reading past the end.</summary>
    internal sealed class NifCursor
    {
        readonly byte[] _data;
        readonly string _entry;

        public NifCursor(byte[] data, int position, string entry)
        {
            _data = data;
            Position = position;
            _entry = entry;
        }

        public int Position { get; private set; }

        public void Require(int count)
        {
            if (count < 0 || Position + (long)count > _data.Length)
                throw new BtrFormatException(_entry, $"read of {count} bytes at offset {Position} runs past the end of the file");
        }

        public void Skip(int count)
        {
            Require(count);
            Position += count;
        }

        public ReadOnlySpan<byte> Slice(int count)
        {
            Require(count);
            var span = new ReadOnlySpan<byte>(_data, Position, count);
            Position += count;
            return span;
        }

        public byte U8() => Slice(1)[0];
        public ushort U16() => BinaryPrimitives.ReadUInt16LittleEndian(Slice(2));
        public uint U32() => BinaryPrimitives.ReadUInt32LittleEndian(Slice(4));
        public int I32() => BinaryPrimitives.ReadInt32LittleEndian(Slice(4));
        public ulong U64() => BinaryPrimitives.ReadUInt64LittleEndian(Slice(8));
        public float F32() => BinaryPrimitives.ReadSingleLittleEndian(Slice(4));

        public string SizedString()
        {
            int length = checked((int)U32());
            return Encoding.ASCII.GetString(Slice(length));
        }
    }
}
