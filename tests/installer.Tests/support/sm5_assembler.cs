using System.Buffers.Binary;
using System.Text;
using AssetConverter.ShaderTranslation.Dxbc;

namespace InstallerTests.Support
{
    /// <summary>Hand-assembles SM5 token streams and wraps them in a DXBC container (synthetic data only).</summary>
    internal sealed class Sm5Assembler
    {
        private readonly List<uint> _body = [];
        private readonly ShaderStage _stage;
        private readonly List<(string Fourcc, byte[] Data)> _extraChunks = [];

        public Sm5Assembler(ShaderStage stage) => _stage = stage;

        public const int X = 0, Y = 1, Z = 2, W = 3;

        public static uint[] Swz(int x, int y, int z, int w) => [(uint)(x | y << 2 | z << 4 | w << 6)];

        public static uint OperandToken(OperandType type, int dims, int numComponents, int mode, int componentBits, int[] repr, bool extended = false)
        {
            uint token = (uint)(numComponents | mode << 2 | componentBits << 4 | (int)type << 12 | dims << 20);
            for (int i = 0; i < repr.Length; i++) token |= (uint)repr[i] << (22 + 3 * i);
            return extended ? token | 0x80000000u : token;
        }

        public static uint[] Dst(OperandType type, int index, int mask) =>
            [OperandToken(type, 1, 2, 0, mask, [0]), (uint)index];

        public static uint[] Src(OperandType type, int index, uint[] swizzle, OperandModifier modifier = OperandModifier.None)
        {
            bool ext = modifier != OperandModifier.None;
            var tokens = new List<uint> { OperandToken(type, 1, 2, 1, (int)swizzle[0], [0], ext) };
            if (ext) tokens.Add(1u | (uint)modifier << 6);
            tokens.Add((uint)index);
            return [.. tokens];
        }

        public static uint[] Scalar(OperandType type, int index, int component) =>
            [OperandToken(type, 1, 2, 2, component, [0]), (uint)index];

        public static uint[] Imm(params float[] values) =>
            [OperandToken(OperandType.Immediate32, 0, 2, 0, 0xF, []), .. values.Select(BitConverter.SingleToUInt32Bits)];

        public static uint[] ImmBits(params uint[] values) =>
            [OperandToken(OperandType.Immediate32, 0, 2, 0, 0xF, []), .. values];

        public static uint[] ConstantBuffer(int register, int element, uint[] swizzle) =>
            [OperandToken(OperandType.ConstantBuffer, 2, 2, 1, (int)swizzle[0], [0, 0]), (uint)register, (uint)element];

        public static uint[] ConstantBufferRelative(int register, uint[] relative, int offset, uint[] swizzle) =>
            [OperandToken(OperandType.ConstantBuffer, 2, 2, 1, (int)swizzle[0], [0, 3]), (uint)register, (uint)offset, .. relative];

        public static uint[] Resource(int register, uint[] swizzle) =>
            [OperandToken(OperandType.Resource, 1, 2, 1, (int)swizzle[0], [0]), (uint)register];

        public static uint[] Sampler(int register) =>
            [OperandToken(OperandType.Sampler, 1, 0, 0, 0, [0]), (uint)register];

        public static uint[] NullOperand() => [OperandToken(OperandType.Null, 0, 0, 0, 0, [])];

        public Sm5Assembler Instruction(Opcode op, uint control, params uint[][] operands) =>
            InstructionWithExtensions(op, control, [], operands);

        public Sm5Assembler InstructionWithExtensions(Opcode op, uint control, uint[] extensions, params uint[][] operands)
        {
            int length = 1 + extensions.Length + operands.Sum(o => o.Length);
            uint token = (uint)op | control << 11 | (uint)length << 24;
            if (extensions.Length > 0) token |= 0x80000000u;
            _body.Add(token);
            _body.AddRange(extensions);
            foreach (var o in operands) _body.AddRange(o);
            return this;
        }

        public Sm5Assembler Declaration(Opcode op, uint control, params uint[][] operandsAndData) =>
            Instruction(op, control, operandsAndData);

        public Sm5Assembler Temps(int count) => Declaration(Opcode.DclTemps, 0, [(uint)count]);

        public Sm5Assembler ConstantBufferDeclaration(int register, int size) =>
            Declaration(Opcode.DclConstantBuffer, 0, [OperandToken(OperandType.ConstantBuffer, 2, 2, 1, 0xE4, [0, 0]), (uint)register, (uint)size]);

        public Sm5Assembler InputDeclaration(int register, int mask, Opcode op = Opcode.DclInput, uint control = 0) =>
            Declaration(op, control, Dst(OperandType.Input, register, mask));

        public Sm5Assembler OutputDeclaration(int register, int mask) =>
            Declaration(Opcode.DclOutput, 0, Dst(OperandType.Output, register, mask));

        public Sm5Assembler SamplerDeclaration(int register, uint mode = 0) =>
            Declaration(Opcode.DclSampler, mode, Sampler(register));

        public Sm5Assembler ResourceDeclaration(int register, ResourceDimension dim, uint returnTypeToken = 0x5555) =>
            Declaration(Opcode.DclResource, (uint)dim, [OperandToken(OperandType.Resource, 1, 0, 0, 0, [0]), (uint)register], [returnTypeToken]);

        public Sm5Assembler RawWords(params uint[] words)
        {
            _body.AddRange(words);
            return this;
        }

        public Sm5Assembler Chunk(string fourcc, byte[] data)
        {
            _extraChunks.Add((fourcc, data));
            return this;
        }

        public byte[] BuildTokens()
        {
            var tokens = new List<uint> { (uint)_stage << 16 | 5u << 4, (uint)(_body.Count + 2) };
            tokens.AddRange(_body);
            var bytes = new byte[tokens.Count * 4];
            for (int i = 0; i < tokens.Count; i++) BinaryPrimitives.WriteUInt32LittleEndian(bytes.AsSpan(i * 4), tokens[i]);
            return bytes;
        }

        public byte[] BuildDxbc() => WrapContainer([("SHEX", BuildTokens()), .. _extraChunks]);

        public static byte[] WrapContainer(IReadOnlyList<(string Fourcc, byte[] Data)> chunks)
        {
            int tableEnd = 32 + 4 * chunks.Count;
            int total = tableEnd + chunks.Sum(c => 8 + c.Data.Length);
            var bytes = new byte[total];
            Encoding.ASCII.GetBytes("DXBC").CopyTo(bytes, 0);
            BinaryPrimitives.WriteUInt32LittleEndian(bytes.AsSpan(20), 1);
            BinaryPrimitives.WriteUInt32LittleEndian(bytes.AsSpan(24), (uint)total);
            BinaryPrimitives.WriteUInt32LittleEndian(bytes.AsSpan(28), (uint)chunks.Count);
            int offset = tableEnd;
            for (int i = 0; i < chunks.Count; i++)
            {
                BinaryPrimitives.WriteUInt32LittleEndian(bytes.AsSpan(32 + 4 * i), (uint)offset);
                Encoding.ASCII.GetBytes(chunks[i].Fourcc).CopyTo(bytes, offset);
                BinaryPrimitives.WriteUInt32LittleEndian(bytes.AsSpan(offset + 4), (uint)chunks[i].Data.Length);
                chunks[i].Data.CopyTo(bytes, offset + 8);
                offset += 8 + chunks[i].Data.Length;
            }
            return bytes;
        }

        /// <summary>ISGN-style signature chunk: count, flags, then 24-byte elements and the name strings.</summary>
        public static byte[] Signature(params (string Name, uint Index, uint SystemValue, uint Register, byte Mask)[] elements)
        {
            int namesStart = 8 + 24 * elements.Length;
            var names = new List<byte>();
            var bytes = new List<byte>();
            bytes.AddRange(BitConverter.GetBytes((uint)elements.Length));
            bytes.AddRange(BitConverter.GetBytes(8u));
            foreach (var e in elements)
            {
                bytes.AddRange(BitConverter.GetBytes((uint)(namesStart + names.Count)));
                names.AddRange(Encoding.ASCII.GetBytes(e.Name));
                names.Add(0);
                bytes.AddRange(BitConverter.GetBytes(e.Index));
                bytes.AddRange(BitConverter.GetBytes(e.SystemValue));
                bytes.AddRange(BitConverter.GetBytes(3u));
                bytes.AddRange(BitConverter.GetBytes(e.Register));
                bytes.Add(e.Mask);
                bytes.Add(e.Mask);
                bytes.AddRange(new byte[2]);
            }
            bytes.AddRange(names);
            return [.. bytes];
        }
    }
}
