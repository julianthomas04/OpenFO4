using AssetConverter.ShaderTranslation;
using AssetConverter.ShaderTranslation.Dxbc;
using InstallerTests.Support;
using Xunit;
using static InstallerTests.Support.Sm5Assembler;

namespace InstallerTests.ShaderTranslation
{
    public class Sm5DecoderTests
    {
        private static string Disassemble(Sm5Assembler asm) =>
            Sm5Disassembler.Disassemble(Sm5Decoder.Decode(asm.BuildDxbc()));

        private static Sm5Assembler Pixel() => new(ShaderStage.Pixel);

        [Fact]
        public void Decode_ReadsStageAndVersion()
        {
            var program = Sm5Decoder.Decode(new Sm5Assembler(ShaderStage.Vertex).Instruction(Opcode.Ret, 0).BuildDxbc());

            Assert.Equal(ShaderStage.Vertex, program.Stage);
            Assert.Equal((5, 0), (program.Major, program.Minor));
            Assert.Equal(Opcode.Ret, Assert.Single(program.Instructions).Opcode);
        }

        [Fact]
        public void Decode_AluInstruction_ProducesOperandsWithSwizzleAndMask()
        {
            var asm = Pixel().Temps(2)
                .Instruction(Opcode.Add, 0, Dst(OperandType.Temp, 0, 0b0111), Src(OperandType.Temp, 1, Swz(X, Y, Z, X)), Imm(1f, 2f, 3f, 4f))
                .Instruction(Opcode.Ret, 0);

            var program = Sm5Decoder.Decode(asm.BuildDxbc());
            var add = program.Instructions[1];

            Assert.Equal(Opcode.Add, add.Opcode);
            Assert.Equal(0b0111, add.Operands[0].Mask);
            Assert.Equal([0, 1, 2, 0], add.Operands[1].Swizzle);
            Assert.True(add.Operands[2].IsImmediate);
            Assert.Equal(BitConverter.SingleToUInt32Bits(3f), add.Operands[2].Immediate[2]);
            Assert.Equal(
                "ps_5_0\ndcl_temps 2\nadd r0.xyz, r1.xyzx, l(0x3f800000, 0x40000000, 0x40400000, 0x40800000)\nret\n",
                Sm5Disassembler.Disassemble(program));
        }

        [Fact]
        public void Decode_SaturateAndOperandModifiers_AreRecovered()
        {
            var asm = Pixel().Temps(3)
                .Instruction(Opcode.Mul, 0x4, Dst(OperandType.Temp, 0, 0b0001),
                    Src(OperandType.Temp, 1, Swz(Y, Y, Y, Y), OperandModifier.AbsoluteNegate), Scalar(OperandType.Temp, 2, Z));

            var mul = Sm5Decoder.Decode(asm.BuildDxbc()).Instructions[1];

            Assert.True(mul.Saturate);
            Assert.Equal(OperandModifier.AbsoluteNegate, mul.Operands[1].Modifier);
            Assert.Equal("mul_sat r0.x, -|r1.y|, r2.z", Sm5Disassembler.Format(mul));
        }

        [Fact]
        public void Decode_RelativeConstantBufferIndex_KeepsRegisterOffsetAndRelativeOperand()
        {
            var asm = Pixel().Temps(3)
                .Instruction(Opcode.Mov, 0, Dst(OperandType.Temp, 0, 0xF),
                    ConstantBufferRelative(1, Scalar(OperandType.Temp, 2, X), 3, Swz(X, Y, Z, W)));

            var src = Sm5Decoder.Decode(asm.BuildDxbc()).Instructions[1].Operands[1];

            Assert.Equal(OperandType.ConstantBuffer, src.Type);
            Assert.Equal(1UL, src.Indices[0].Immediate);
            Assert.Equal(3UL, src.Indices[1].Immediate);
            Assert.Equal(OperandType.Temp, src.Indices[1].Relative!.Type);
            Assert.Equal("mov r0.xyzw, cb1[r2.x + 3].xyzw", Sm5Disassembler.Format(Sm5Decoder.Decode(asm.BuildDxbc()).Instructions[1]));
        }

        [Fact]
        public void Decode_SampleWithExtendedTokens_ReadsOffsetAndDimension()
        {
            uint offsetToken = 1u | 1u << 9 | 0xFu << 13 | 0x80000000u;
            uint dimensionToken = 2u | (uint)ResourceDimension.Texture2D << 6;
            var asm = Pixel().Temps(2)
                .InstructionWithExtensions(Opcode.Sample, 0, [offsetToken, dimensionToken],
                    Dst(OperandType.Temp, 0, 0xF), Src(OperandType.Temp, 1, Swz(X, Y, X, X)), Resource(0, Swz(X, Y, Z, W)), Sampler(0));

            var sample = Sm5Decoder.Decode(asm.BuildDxbc()).Instructions[1];

            Assert.True(sample.HasSampleOffset);
            Assert.Equal((1, -1, 0), (sample.OffsetU, sample.OffsetV, sample.OffsetW));
            Assert.Equal(ResourceDimension.Texture2D, sample.ExtendedDimension);
            Assert.Equal("sample_indexable(texture2d)(1,-1,0) r0.xyzw, r1.xyxx, t0.xyzw, s0", Sm5Disassembler.Format(sample));
        }

        [Fact]
        public void Disassemble_StructuredControlFlow_IndentsNestedBlocks()
        {
            var asm = Pixel().Temps(1)
                .Instruction(Opcode.If, 0x80, Scalar(OperandType.Temp, 0, X))
                .Instruction(Opcode.Mov, 0, Dst(OperandType.Temp, 0, 0b0010), Imm(1f, 1f, 1f, 1f))
                .Instruction(Opcode.Else, 0)
                .Instruction(Opcode.Discard, 0, Scalar(OperandType.Temp, 0, Y))
                .Instruction(Opcode.Endif, 0)
                .Instruction(Opcode.Ret, 0);

            Assert.Equal(
                "ps_5_0\ndcl_temps 1\nif_nz r0.x\n    mov r0.y, l(0x3f800000, 0x3f800000, 0x3f800000, 0x3f800000)\nelse\n    discard_z r0.y\nendif\nret\n",
                Disassemble(asm));
        }

        [Fact]
        public void Decode_Declarations_RoundTripThroughDisassembly()
        {
            var asm = Pixel()
                .ConstantBufferDeclaration(2, 5)
                .SamplerDeclaration(0, mode: 1)
                .ResourceDeclaration(3, ResourceDimension.Texture2DArray)
                .InputDeclaration(1, 0b0111, Opcode.DclInputPs, control: (uint)InterpolationMode.Linear)
                .OutputDeclaration(0, 0xF);

            Assert.Equal(
                "ps_5_0\ndcl_constantBuffer cb2[5], immediateIndexed\ndcl_sampler s0, comparisonMode\n"
                + "dcl_resource_texture2darray (float,float,float,float) t3\ndcl_input_ps linear v1.xyz\ndcl_output o0.xyzw\n",
                Disassemble(asm));
        }

        [Fact]
        public void Decode_ImmediateConstantBuffer_ReturnsRawDwords()
        {
            var asm = Pixel().RawWords(53u | 3u << 11, 10, 1, 2, 3, 4, 5, 6, 7, 8);

            var data = Sm5Decoder.Decode(asm.BuildDxbc()).Instructions[0];

            Assert.Equal(Opcode.CustomData, data.Opcode);
            Assert.Equal<uint>([1, 2, 3, 4, 5, 6, 7, 8], data.Data);
        }

        [Fact]
        public void Decode_Signatures_AreReadFromContainerChunks()
        {
            var asm = new Sm5Assembler(ShaderStage.Vertex).Instruction(Opcode.Ret, 0)
                .Chunk("ISGN", Signature(("POSITION", 0, 0, 0, 0x7), ("TEXCOORD", 1, 0, 1, 0x3)))
                .Chunk("OSGN", Signature(("SV_Position", 0, 1, 0, 0xF)));

            var program = Sm5Decoder.Decode(asm.BuildDxbc());

            Assert.Equal(["POSITION", "TEXCOORD"], program.Inputs.Select(i => i.SemanticName));
            Assert.Equal<uint>([0, 1], program.Inputs.Select(i => i.Register));
            Assert.Equal(1u, program.Outputs[0].SystemValue);
            Assert.Equal(0xF, program.Outputs[0].Mask);
        }

        [Fact]
        public void Decode_UnknownOpcode_ThrowsWithTokenOffset()
        {
            var asm = Pixel().RawWords(0x7FFu | 1u << 24);

            var ex = Assert.Throws<Sm5DecodeException>(() => Sm5Decoder.Decode(asm.BuildDxbc()));
            Assert.Equal(2, ex.TokenOffset);
        }

        [Fact]
        public void Decode_ZeroLengthInstruction_Throws()
        {
            Assert.Throws<Sm5DecodeException>(() => Sm5Decoder.Decode(Pixel().RawWords((uint)Opcode.Add).BuildDxbc()));
        }

        [Fact]
        public void Decode_InstructionPastEndOfProgram_Throws()
        {
            var asm = Pixel().RawWords((uint)Opcode.Mov | 9u << 24);

            Assert.Throws<Sm5DecodeException>(() => Sm5Decoder.Decode(asm.BuildDxbc()));
        }

        [Fact]
        public void Decode_ImmediateOperandCutOff_Throws()
        {
            var asm = Pixel().RawWords(
                (uint)Opcode.Mov | 5u << 24,
                OperandToken(OperandType.Temp, 1, 2, 0, 0xF, [0]), 0,
                OperandToken(OperandType.Immediate32, 0, 2, 0, 0xF, []), 0x3f800000);

            Assert.Throws<Sm5DecodeException>(() => Sm5Decoder.Decode(asm.BuildDxbc()));
        }

        [Fact]
        public void Decode_ProgramLengthExceedingChunk_Throws()
        {
            byte[] tokens = Pixel().Instruction(Opcode.Ret, 0).BuildTokens();
            BitConverter.GetBytes(99u).CopyTo(tokens, 4);

            Assert.Throws<Sm5DecodeException>(() => Sm5Decoder.Decode(WrapContainer([("SHEX", tokens)])));
        }

        [Fact]
        public void Decode_MissingShaderChunk_Throws()
        {
            var ex = Assert.Throws<DxbcFormatException>(() => Sm5Decoder.Decode(WrapContainer([("ISGN", Signature())])));
            Assert.Contains("SHEX", ex.Message);
        }

        [Fact]
        public void Decode_BadMagic_Throws()
        {
            byte[] dxbc = Pixel().Instruction(Opcode.Ret, 0).BuildDxbc();
            dxbc[0] = (byte)'X';

            Assert.Throws<DxbcFormatException>(() => Sm5Decoder.Decode(dxbc));
        }

        [Fact]
        public void Decode_ChunkOverrunningContainer_Throws()
        {
            byte[] dxbc = Pixel().Instruction(Opcode.Ret, 0).BuildDxbc();
            BitConverter.GetBytes(0x10000u).CopyTo(dxbc, 40);

            Assert.Throws<DxbcFormatException>(() => Sm5Decoder.Decode(dxbc));
        }

        [Fact]
        public void Decode_UnknownProgramType_Throws()
        {
            byte[] tokens = Pixel().Instruction(Opcode.Ret, 0).BuildTokens();
            BitConverter.GetBytes(9u << 16 | 5u << 4).CopyTo(tokens, 0);

            Assert.Throws<DxbcFormatException>(() => Sm5Decoder.Decode(WrapContainer([("SHEX", tokens)])));
        }

        [Fact]
        public void OpcodeTable_CoversEveryOpcodeEnumMember()
        {
            Assert.Equal(Enum.GetValues<Opcode>().Length, OpcodeInfo.Count);
            Assert.Equal("sample_c_lz", OpcodeInfo.Name(Opcode.SampleCLz));
            Assert.Equal(2, OpcodeInfo.DestinationCount(Opcode.Sincos));
            Assert.Equal(0, OpcodeInfo.DestinationCount(Opcode.If));
        }
    }
}
