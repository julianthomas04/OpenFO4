using System.Buffers.Binary;

namespace AssetConverter.ShaderTranslation.Dxbc
{
    /// <summary>Decodes the SHDR/SHEX token stream of a DXBC container (SM4/SM5) into <see cref="Sm5Program"/>.</summary>
    public static class Sm5Decoder
    {
        private const int MaxInstructionLength = 127;

        public static Sm5Program Decode(ReadOnlyMemory<byte> dxbc)
        {
            var container = DxbcContainer.Parse(dxbc);
            var shex = container.RequireChunk("SHEX", "SHDR").Span;
            if (shex.Length % 4 != 0)
                throw new DxbcFormatException("Shader chunk size is not a multiple of 4.");
            var tokens = new uint[shex.Length / 4];
            for (int i = 0; i < tokens.Length; i++)
                tokens[i] = BinaryPrimitives.ReadUInt32LittleEndian(shex[(4 * i)..]);

            return new Sm5Program
            {
                Stage = ReadStage(tokens),
                Major = (int)(tokens[0] >> 4 & 0xF),
                Minor = (int)(tokens[0] & 0xF),
                Instructions = new TokenReader(tokens).ReadInstructions(),
                Inputs = container.ReadSignature("ISGN", "ISG1"),
                Outputs = container.ReadSignature("OSGN", "OSG1"),
                PatchConstants = container.ReadSignature("PCSG", "PSG1"),
            };
        }

        private static ShaderStage ReadStage(uint[] tokens)
        {
            if (tokens.Length < 2)
                throw new DxbcFormatException("Shader chunk has no version/length tokens.");
            uint type = tokens[0] >> 16;
            if (type > (uint)ShaderStage.Compute)
                throw new DxbcFormatException($"Unknown shader program type {type}.");
            return (ShaderStage)type;
        }

        private sealed class TokenReader
        {
            private readonly uint[] _t;
            private readonly int _programEnd;

            public TokenReader(uint[] tokens)
            {
                _t = tokens;
                if (tokens[1] > tokens.Length)
                    throw new Sm5DecodeException(1, $"Program length {tokens[1]} exceeds the chunk ({tokens.Length} tokens).");
                _programEnd = (int)tokens[1];
            }

            public List<Instruction> ReadInstructions()
            {
                var list = new List<Instruction>();
                int pos = 2;
                while (pos < _programEnd)
                    list.Add(ReadInstruction(ref pos));
                return list;
            }

            private Instruction ReadInstruction(ref int pos)
            {
                int start = pos;
                uint token = _t[pos];
                uint rawOpcode = token & 0x7FF;
                if (!OpcodeInfo.IsKnown(rawOpcode))
                    throw new Sm5DecodeException(start, $"Unknown opcode {rawOpcode}.");
                var opcode = (Opcode)rawOpcode;
                uint control = token >> 11 & 0x1FFF;

                if (opcode == Opcode.CustomData)
                    return ReadCustomData(ref pos, control);

                int length = (int)(token >> 24 & 0x7F);
                if (length == 0 || length > MaxInstructionLength)
                    throw new Sm5DecodeException(start, $"Invalid instruction length {length} for {OpcodeInfo.Name(opcode)}.");
                int end = start + length;
                if (end > _programEnd)
                    throw new Sm5DecodeException(start, $"Instruction {OpcodeInfo.Name(opcode)} runs past the end of the program.");

                int p = start + 1;
                var ext = ReadExtendedOpcodes(token, ref p, end);

                var (operandCount, dwordCount) = DeclarationShape(opcode);
                var operands = new List<Operand>();
                while (p < end && operands.Count < operandCount)
                    operands.Add(ReadOperand(ref p, end));
                int dwordsTaken = Math.Min(end - p, dwordCount);
                var data = new List<uint>();
                for (int i = 0; i < dwordsTaken; i++)
                    data.Add(_t[p++]);
                if (!OpcodeInfo.IsDeclaration(opcode))
                    while (p < end)
                        operands.Add(ReadOperand(ref p, end));
                while (p < end)
                    data.Add(_t[p++]);

                if (data.Count < dwordCount)
                    throw new Sm5DecodeException(start, $"{OpcodeInfo.Name(opcode)} declaration is missing {dwordCount - data.Count} data dword(s).");
                if (OpcodeInfo.ExpectedOperandCount(opcode) is int expected && operands.Count != expected)
                    throw new Sm5DecodeException(start, $"{OpcodeInfo.Name(opcode)} takes {expected} operand(s) but the instruction has {operands.Count}.");

                pos = end;
                return new Instruction
                {
                    Opcode = opcode,
                    Control = control,
                    Operands = operands,
                    Data = data,
                    HasSampleOffset = ext.HasOffset,
                    OffsetU = ext.U,
                    OffsetV = ext.V,
                    OffsetW = ext.W,
                    ExtendedDimension = ext.Dimension,
                    ExtendedStride = ext.Stride,
                    ExtendedReturnType = ext.ReturnType,
                    ByteOffset = start * 4,
                };
            }

            private Instruction ReadCustomData(ref int pos, uint control)
            {
                int start = pos;
                if (start + 1 >= _programEnd)
                    throw new Sm5DecodeException(start, "Truncated customdata header.");
                int total = (int)_t[start + 1];
                if (total < 2 || start + total > _programEnd)
                    throw new Sm5DecodeException(start, $"Invalid customdata length {total}.");
                var data = new uint[total - 2];
                Array.Copy(_t, start + 2, data, 0, data.Length);
                pos = start + total;
                return new Instruction { Opcode = Opcode.CustomData, Control = control, Data = data, ByteOffset = start * 4 };
            }

            private (bool HasOffset, sbyte U, sbyte V, sbyte W, ResourceDimension Dimension, uint Stride, ResourceReturnType[] ReturnType)
                ReadExtendedOpcodes(uint token, ref int p, int end)
            {
                bool hasOffset = false;
                sbyte u = 0, v = 0, w = 0;
                var dim = ResourceDimension.Unknown;
                uint stride = 0;
                ResourceReturnType[] ret = [];
                while ((token >> 31) != 0)
                {
                    if (p >= end)
                        throw new Sm5DecodeException(p, "Extended opcode token runs past the instruction.");
                    token = _t[p++];
                    switch (token & 0x3F)
                    {
                        case 1:
                            hasOffset = true;
                            u = SignExtend4(token >> 9);
                            v = SignExtend4(token >> 13);
                            w = SignExtend4(token >> 17);
                            break;
                        case 2:
                            dim = (ResourceDimension)(token >> 6 & 0x1F);
                            stride = token >> 11 & 0x1FF;
                            break;
                        case 3:
                            ret = Enumerable.Range(0, 4).Select(i => (ResourceReturnType)(token >> (6 + 4 * i) & 0xF)).ToArray();
                            break;
                    }
                }
                return (hasOffset, u, v, w, dim, stride, ret);
            }

            private static sbyte SignExtend4(uint v) => (sbyte)((int)(v & 0xF) << 28 >> 28);

            private static (int Operands, int Dwords) DeclarationShape(Opcode op) => op switch
            {
                Opcode.DclResource or Opcode.DclUavTyped => (1, 1),
                Opcode.DclResourceStructured or Opcode.DclUavStructured or Opcode.DclTgsmRaw => (1, 1),
                Opcode.DclTgsmStructured => (1, 2),
                Opcode.DclIndexRange => (1, 1),
                Opcode.DclInputSgv or Opcode.DclInputSiv or Opcode.DclInputPsSgv or Opcode.DclInputPsSiv
                    or Opcode.DclOutputSgv or Opcode.DclOutputSiv => (1, 1),
                Opcode.DclConstantBuffer or Opcode.DclSampler or Opcode.DclInput or Opcode.DclInputPs
                    or Opcode.DclOutput or Opcode.DclResourceRaw or Opcode.DclUavRaw or Opcode.DclStream => (1, 0),
                Opcode.DclTemps or Opcode.DclHsMaxTessfactor or Opcode.DclHsForkPhaseInstanceCount
                    or Opcode.DclHsJoinPhaseInstanceCount or Opcode.DclMaxOutputVertexCount
                    or Opcode.DclGsInstanceCount or Opcode.DclFunctionBody => (0, 1),
                Opcode.DclIndexableTemp or Opcode.DclThreadGroup => (0, 3),
                _ => (0, 0),
            };

            private Operand ReadOperand(ref int p, int end)
            {
                if (p >= end)
                    throw new Sm5DecodeException(p, "Operand token runs past the instruction.");
                int tokenPos = p;
                uint token = _t[p++];
                int numComponents = (int)(token & 3);
                var mode = (SelectionMode)(token >> 2 & 3);
                if (numComponents == 3)
                    throw new Sm5DecodeException(tokenPos, "N-component operands are not supported.");
                int componentCount = numComponents switch { 0 => 0, 1 => 1, _ => 4 };
                var type = (OperandType)(token >> 12 & 0xFF);
                if (!Enum.IsDefined(type))
                    throw new Sm5DecodeException(tokenPos, $"Unknown operand type {(int)type}.");
                int indexDim = (int)(token >> 20 & 3);

                var modifier = OperandModifier.None;
                int minPrecision = 0;
                bool nonUniform = false;
                uint extToken = token;
                while ((extToken >> 31) != 0)
                {
                    if (p >= end)
                        throw new Sm5DecodeException(p, "Extended operand token runs past the instruction.");
                    extToken = _t[p++];
                    if ((extToken & 0x3F) == 1)
                    {
                        modifier = (OperandModifier)(extToken >> 6 & 0xFF);
                        minPrecision = (int)(extToken >> 14 & 7);
                        nonUniform = (extToken >> 17 & 1) != 0;
                    }
                }

                var swizzle = new byte[] { 0, 1, 2, 3 };
                byte mask = 0, select1 = 0;
                if (componentCount == 4)
                {
                    switch (mode)
                    {
                        case SelectionMode.Mask: mask = (byte)(token >> 4 & 0xF); break;
                        case SelectionMode.Swizzle:
                            for (int i = 0; i < 4; i++) swizzle[i] = (byte)(token >> (4 + 2 * i) & 3);
                            break;
                        case SelectionMode.Select1: select1 = (byte)(token >> 4 & 3); break;
                        default: throw new Sm5DecodeException(tokenPos, "Invalid selection mode.");
                    }
                }

                var indices = new List<OperandIndex>();
                for (int dim = 0; dim < indexDim; dim++)
                    indices.Add(ReadIndex((IndexRepresentation)(token >> (22 + 3 * dim) & 7), ref p, end));

                var immediate = new List<uint>();
                if (type is OperandType.Immediate32 or OperandType.Immediate64)
                {
                    int count = Math.Max(componentCount, 1) * (type == OperandType.Immediate64 ? 2 : 1);
                    if (p + count > end)
                        throw new Sm5DecodeException(p, "Immediate operand runs past the instruction.");
                    for (int i = 0; i < count; i++) immediate.Add(_t[p++]);
                }

                return new Operand
                {
                    Type = type,
                    ComponentCount = componentCount,
                    Mode = mode,
                    Mask = mask,
                    Swizzle = swizzle,
                    Select1 = select1,
                    Indices = indices,
                    Modifier = modifier,
                    MinPrecision = minPrecision,
                    NonUniform = nonUniform,
                    Immediate = immediate,
                };
            }

            private OperandIndex ReadIndex(IndexRepresentation repr, ref int p, int end)
            {
                ulong imm = 0;
                Operand? relative = null;
                switch (repr)
                {
                    case IndexRepresentation.Immediate32:
                    case IndexRepresentation.Immediate32PlusRelative:
                        imm = TakeDword(ref p, end);
                        break;
                    case IndexRepresentation.Immediate64:
                    case IndexRepresentation.Immediate64PlusRelative:
                        imm = TakeDword(ref p, end) | (ulong)TakeDword(ref p, end) << 32;
                        break;
                    case IndexRepresentation.Relative:
                        break;
                    default:
                        throw new Sm5DecodeException(p, $"Invalid index representation {(int)repr}.");
                }
                if (repr is IndexRepresentation.Relative or IndexRepresentation.Immediate32PlusRelative or IndexRepresentation.Immediate64PlusRelative)
                    relative = ReadOperand(ref p, end);
                return new OperandIndex { Representation = repr, Immediate = imm, Relative = relative };
            }

            private uint TakeDword(ref int p, int end)
            {
                if (p >= end)
                    throw new Sm5DecodeException(p, "Index runs past the instruction.");
                return _t[p++];
            }
        }
    }
}
