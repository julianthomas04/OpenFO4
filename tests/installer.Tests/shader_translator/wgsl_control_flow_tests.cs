using AssetConverter.ShaderTranslation;
using AssetConverter.ShaderTranslation.Dxbc;
using InstallerTests.Support;
using Xunit;
using static InstallerTests.ShaderTranslation.WgslTestKit;
using static InstallerTests.Support.Sm5Assembler;

namespace InstallerTests.ShaderTranslation
{
    public class WgslControlFlowTests
    {
        private const uint TestNonZero = 0x80;
        private const string DiscardEpilogue = "if (discard_pending) { discard; }";

        private static Sm5Assembler Textured() => Pixel().SamplerDeclaration(0).ResourceDeclaration(0, ResourceDimension.Texture2D).Temps(3);

        [Fact]
        public void Discard_OnlySetsAFlagSoLaterSamplesKeepValidDerivatives()
        {
            var asm = Textured()
                .Instruction(Opcode.Discard, TestNonZero, RegScalar(0, X))
                .Instruction(Opcode.Sample, 0, Temp(1), Reg(0), Resource(0, AllComponents), Sampler(0))
                .Instruction(Opcode.Ret, 0);

            var statements = Statements(Body(asm));

            int flagSet = Array.IndexOf(statements, "discard_pending = true;");
            int sample = Array.FindIndex(statements, s => s.Contains("textureSample("));
            int realDiscard = Array.IndexOf(statements, DiscardEpilogue);
            Assert.InRange(flagSet, 0, int.MaxValue);
            Assert.True(flagSet < sample, "the flag is set before the later sample");
            Assert.True(sample < realDiscard, "the real discard happens after the later sample");
            Assert.Contains("var discard_pending: bool = false;", statements);
            Assert.Equal(DiscardEpilogue, statements[^3]);
            Assert.Equal("return o;", statements[^2]);
        }

        [Fact]
        public void Discard_IsAppliedOnEveryReturnPath()
        {
            var asm = Pixel().Temps(2)
                .Instruction(Opcode.Discard, TestNonZero, RegScalar(0, X))
                .Instruction(Opcode.Retc, TestNonZero, RegScalar(0, Y))
                .Instruction(Opcode.Mov, 0, Temp(1), Reg(0))
                .Instruction(Opcode.Ret, 0);

            var statements = Statements(Body(asm));

            Assert.Equal(2, statements.Count(s => s == DiscardEpilogue));
            Assert.Equal(2, statements.Count(s => s == "return o;"));
            Assert.All(Enumerable.Range(0, statements.Length).Where(i => statements[i] == "return o;"),
                i => Assert.Equal(DiscardEpilogue, statements[i - 1]));
        }

        [Fact]
        public void NoDiscardInstruction_EmitsNoDiscardMachinery()
        {
            var asm = Pixel().Temps(1).Instruction(Opcode.Mov, 0, Temp(0), Reg(0)).Instruction(Opcode.Ret, 0);

            string wgsl = Body(asm);

            Assert.DoesNotContain("discard", wgsl);
        }

        [Fact]
        public void Discard_PolarityFollowsTheTestFlag()
        {
            var asm = Pixel().Temps(1)
                .Instruction(Opcode.Discard, 0, RegScalar(0, X))
                .Instruction(Opcode.Ret, 0);

            Assert.Contains("if (bitcast<u32>(r0.x) == 0u) {", Statements(Body(asm)));
        }

        [Fact]
        public void LoopControl_BreakAndContinueStayDistinct()
        {
            var asm = Pixel().Temps(1)
                .Instruction(Opcode.Loop, 0)
                .Instruction(Opcode.Continuec, TestNonZero, RegScalar(0, X))
                .Instruction(Opcode.Breakc, 0, RegScalar(0, Y))
                .Instruction(Opcode.Endloop, 0)
                .Instruction(Opcode.Ret, 0);

            var statements = Statements(Body(asm));

            Assert.Equal("continue;", statements[Array.IndexOf(statements, "if (bitcast<u32>(r0.x) != 0u) {") + 1]);
            Assert.Equal("break;", statements[Array.IndexOf(statements, "if (bitcast<u32>(r0.y) == 0u) {") + 1]);
        }

        [Fact]
        public void SwitchCaseLabels_KeepTheirSign()
        {
            var asm = Pixel().Temps(1)
                .Instruction(Opcode.Switch, 0, RegScalar(0, X))
                .Instruction(Opcode.Case, 0, ImmBits(0xFFFFFFFFu, 0, 0, 0))
                .Instruction(Opcode.Break, 0)
                .Instruction(Opcode.Default, 0)
                .Instruction(Opcode.Break, 0)
                .Instruction(Opcode.Endswitch, 0)
                .Instruction(Opcode.Ret, 0);

            var statements = Statements(Body(asm));

            Assert.Contains("case -1: {", statements);
            Assert.Contains("default: {", statements);
        }

        [Fact]
        public void ImmediateConstantBuffer_RelativeRead_IndexesThePrivateArray()
        {
            uint[] icbOperand = [OperandToken(OperandType.ImmediateConstantBuffer, 1, 2, 1, 0xE4, [2]), .. RegScalar(1, X)];
            var asm = Pixel().Temps(2).RawWords(53u | 3u << 11, 10, 0x3f800000, 0, 0, 0, 0x40000000, 0, 0, 0)
                .Instruction(Opcode.Mov, 0, Temp(0), icbOperand)
                .Instruction(Opcode.Ret, 0);

            Assert.Contains("r0 = bitcast<vec4<f32>>(blob_icb[bitcast<u32>(r1.x)]).xyzw;", Statements(Body(asm)));
        }

        [Fact]
        public void IndexableTemp_UsesAnArrayWithTheDeclaredSize()
        {
            var asm = Pixel().Temps(1).Declaration(Opcode.DclIndexableTemp, 0, [2u], [16u], [4u])
                .Instruction(Opcode.Ret, 0);

            Assert.Contains("var x2: array<vec4<f32>, 16>;", Statements(Body(asm)));
        }
    }
}
