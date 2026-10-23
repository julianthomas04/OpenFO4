using AssetConverter.ShaderTranslation.Dxbc;
using AssetConverter.ShaderTranslation.Wgsl;
using InstallerTests.Support;
using Xunit;
using static InstallerTests.ShaderTranslation.WgslTestKit;
using static InstallerTests.Support.Sm5Assembler;

namespace InstallerTests.ShaderTranslation
{
    public class WgslAluTests
    {
        private const uint Saturate = 0x4;

        private static Sm5Assembler Scalar1() => Pixel().Temps(5);

        private static uint[] Bits(uint value) => ImmBits(value, value, value, value);

        private static string[] Emitted(Sm5Assembler asm, WgslEmitOptions? options = null) => Statements(Body(asm, options));

        [Theory]
        [InlineData(Opcode.RoundNe, "round")]
        [InlineData(Opcode.RoundNi, "floor")]
        [InlineData(Opcode.RoundPi, "ceil")]
        [InlineData(Opcode.RoundZ, "trunc")]
        [InlineData(Opcode.Frc, "fract")]
        [InlineData(Opcode.Sqrt, "sqrt")]
        [InlineData(Opcode.Rsq, "inverseSqrt")]
        [InlineData(Opcode.Exp, "exp2")]
        [InlineData(Opcode.Log, "log2")]
        public void UnaryFloatOpcodes_MapToTheMatchingWgslBuiltin(Opcode opcode, string builtin)
        {
            var asm = Scalar1().Instruction(opcode, 0, Temp(0, 1), RegScalar(1, X));

            Assert.Contains($"r0.x = {builtin}(r1.x);", Emitted(asm));
        }

        [Theory]
        [InlineData(Opcode.Ftoi, "r0.x = bitcast<f32>(i32(r1.x));")]
        [InlineData(Opcode.Ftou, "r0.x = bitcast<f32>(u32(r1.x));")]
        [InlineData(Opcode.Itof, "r0.x = f32(bitcast<i32>(r1.x));")]
        [InlineData(Opcode.Utof, "r0.x = f32(bitcast<u32>(r1.x));")]
        public void Conversions_ReinterpretTheSourceWithTheCorrectSignedness(Opcode opcode, string expected)
        {
            var asm = Scalar1().Instruction(opcode, 0, Temp(0, 1), RegScalar(1, X));

            Assert.Contains(expected, Emitted(asm));
        }

        [Fact]
        public void Movc_SelectsTrueOperandWhenConditionIsNonZero()
        {
            var asm = Scalar1().Instruction(Opcode.Movc, 0, Temp(0, 1), RegScalar(1, X), RegScalar(2, X), RegScalar(3, X));

            Assert.Contains("r0.x = select(r3.x, r2.x, (bitcast<u32>(r1.x) != 0u));", Emitted(asm));
        }

        [Fact]
        public void DotProducts_UseTheComponentCountOfTheOpcode()
        {
            var asm = Scalar1()
                .Instruction(Opcode.Dp2, 0, Temp(0, 1), Reg(1), Reg(2))
                .Instruction(Opcode.Dp3, 0, Temp(0, 2), Reg(1), Reg(2))
                .Instruction(Opcode.Dp4, 0, Temp(0, 4), Reg(1), Reg(2));

            var statements = Emitted(asm);

            Assert.Contains("r0.x = dot(r1.xy, r2.xy);", statements);
            Assert.Contains("r0.y = dot(r1.xyz, r2.xyz);", statements);
            Assert.Contains("r0.z = dot(r1.xyzw, r2.xyzw);", statements);
        }

        [Fact]
        public void Dot_WrittenToSeveralComponents_ReplicatesTheScalar()
        {
            var asm = Scalar1().Instruction(Opcode.Dp3, 0, Temp(0), Reg(1), Reg(2));

            Assert.Contains("r0 = vec4<f32>(dot(r1.xyz, r2.xyz));", Emitted(asm));
        }

        [Fact]
        public void Sincos_WritesSineAndCosineToTheirDestinations()
        {
            var asm = Scalar1().Instruction(Opcode.Sincos, 0, Temp(0, 1), Temp(1, 1), RegScalar(2, X));

            var statements = Emitted(asm);

            Assert.Contains("r0.x = sin(r2.x);", statements);
            Assert.Contains("r1.x = cos(r2.x);", statements);
        }

        [Fact]
        public void Sincos_WithNullSineDestination_OnlyComputesCosine()
        {
            var asm = Scalar1().Instruction(Opcode.Sincos, 0, NullOperand(), Temp(1, 1), RegScalar(2, X));

            var statements = Emitted(asm);

            Assert.Contains("r1.x = cos(r2.x);", statements);
            Assert.DoesNotContain(statements, s => s.Contains("sin("));
        }

        [Fact]
        public void HalfFloatConversions_GoThroughPackAndUnpack()
        {
            var asm = Scalar1()
                .Instruction(Opcode.F32ToF16, 0, Temp(0, 1), RegScalar(1, X))
                .Instruction(Opcode.F16ToF32, 0, Temp(1, 1), RegScalar(2, X));

            var statements = Emitted(asm);

            Assert.Contains("r0.x = bitcast<f32>((pack2x16float(vec2<f32>(r1.x, 0.0)) & 0xffffu));", statements);
            Assert.Contains("r1.x = unpack2x16float(bitcast<u32>(r2.x) & 0xffffu).x;", statements);
        }

        [Fact]
        public void FirstbitHi_CountsLeadingZerosAndReturnsAllOnesForZero()
        {
            var asm = Scalar1().Instruction(Opcode.FirstbitHi, 0, Temp(0, 1), RegScalar(1, X));

            Assert.Contains(
                "r0.x = bitcast<f32>(select(31u - firstLeadingBit(bitcast<u32>(r1.x)), 0xffffffffu, bitcast<u32>(r1.x) == 0u));",
                Emitted(asm));
        }

        [Fact]
        public void BitFieldExtract_MasksOffsetAndClampsWidthToTheWord()
        {
            var asm = Scalar1()
                .Instruction(Opcode.Ubfe, 0, Temp(0, 1), RegScalar(1, X), RegScalar(2, X), RegScalar(3, X))
                .Instruction(Opcode.Ibfe, 0, Temp(1, 1), RegScalar(1, X), RegScalar(2, X), RegScalar(3, X));

            var statements = Emitted(asm);

            const string Field = "(bitcast<u32>(r2.x) & 31u), min((bitcast<u32>(r1.x) & 31u), 32u - (bitcast<u32>(r2.x) & 31u)))";
            Assert.Contains($"r0.x = bitcast<f32>(extractBits(bitcast<u32>(r3.x), {Field});", statements);
            Assert.Contains($"r1.x = bitcast<f32>(extractBits(bitcast<i32>(r3.x), {Field});", statements);
        }

        [Fact]
        public void BitFieldInsert_PlacesTheSourceIntoTheBase()
        {
            var asm = Scalar1().Instruction(Opcode.Bfi, 0, Temp(0, 1), RegScalar(1, X), RegScalar(2, X), RegScalar(3, X), RegScalar(4, X));

            Assert.Contains(
                "r0.x = bitcast<f32>(insertBits(bitcast<u32>(r4.x), bitcast<u32>(r3.x), (bitcast<u32>(r2.x) & 31u), min((bitcast<u32>(r1.x) & 31u), 32u - (bitcast<u32>(r2.x) & 31u))));",
                Emitted(asm));
        }

        [Theory]
        [InlineData(Opcode.Ishl, "<<", "bitcast<i32>")]
        [InlineData(Opcode.Ishr, ">>", "bitcast<i32>")]
        [InlineData(Opcode.Ushr, ">>", "bitcast<u32>")]
        public void Shifts_MaskLiteralCountsToFiveBitsLikeD3d(Opcode opcode, string op, string cast)
        {
            var asm = Scalar1().Instruction(opcode, 0, Temp(0, 1), RegScalar(1, X), Bits(35));

            Assert.Contains($"r0.x = bitcast<f32>(({cast}(r1.x) {op} 3u));", Emitted(asm));
        }

        [Fact]
        public void Shifts_ByRegisterCount_RelyOnWgslModuloSemantics()
        {
            var asm = Scalar1().Instruction(Opcode.Ishl, 0, Temp(0, 1), RegScalar(1, X), RegScalar(2, X));

            Assert.Contains("r0.x = bitcast<f32>((bitcast<i32>(r1.x) << bitcast<u32>(r2.x)));", Emitted(asm));
        }

        [Fact]
        public void LiteralOnlyArithmetic_IsHiddenFromConstantEvaluation()
        {
            var asm = Scalar1()
                .Instruction(Opcode.Iadd, 0, Temp(0, 1), Bits(int.MaxValue), Bits(1))
                .Instruction(Opcode.Div, 0, Temp(1, 1), Imm(1f, 1f, 1f, 1f), Imm(0f, 0f, 0f, 0f))
                .Instruction(Opcode.Rcp, 0, Temp(2, 1), Imm(0f, 0f, 0f, 0f))
                .Instruction(Opcode.Ishl, 0, Temp(3, 1), Bits(1), Bits(40));

            var statements = Emitted(asm);

            Assert.Contains("var tmp0 = 2147483647i;", statements);
            Assert.Contains("var tmp1 = 1i;", statements);
            Assert.Contains("r0.x = bitcast<f32>((tmp0 + tmp1));", statements);
            Assert.Contains("r1.x = (tmp2 / tmp3);", statements);
            Assert.Contains("var tmp3 = 0.0;", statements);
            Assert.Contains("r2.x = (1.0 / tmp4);", statements);
            Assert.Contains("r3.x = bitcast<f32>((tmp5 << tmp6));", statements);
            Assert.Contains("var tmp6 = 8u;", statements);
        }

        [Fact]
        public void LiteralWithRegisterOperand_IsLeftForTheCompilerToFold()
        {
            var asm = Scalar1().Instruction(Opcode.Add, 0, Temp(0, 1), RegScalar(1, X), Imm(2f, 2f, 2f, 2f));

            var statements = Emitted(asm);

            Assert.Contains("r0.x = (r1.x + 2.0);", statements);
            Assert.DoesNotContain(statements, s => s.StartsWith("var tmp", StringComparison.Ordinal));
        }

        [Fact]
        public void UnsignedDivision_ByLiteralZero_UsesARuntimeDivisorAndYieldsAllOnes()
        {
            var asm = Scalar1().Instruction(Opcode.Udiv, 0, Temp(0, 1), Temp(1, 1), RegScalar(2, X), Bits(0));

            var statements = Emitted(asm);

            Assert.Contains("let tmp0 = bitcast<u32>(r2.x);", statements);
            Assert.Contains("var tmp1 = 0u;", statements);
            Assert.Contains("r0.x = bitcast<f32>(select(tmp0 / tmp1, 0xffffffffu, tmp1 == 0u));", statements);
            Assert.Contains("r1.x = bitcast<f32>(select(tmp0 % tmp1, 0xffffffffu, tmp1 == 0u));", statements);
        }

        [Fact]
        public void IntegerAbsolute_AppliesToUnsignedReadsAsSignedValue()
        {
            var asm = Scalar1().Instruction(Opcode.And, 0, Temp(0, 1), Src(OperandType.Temp, 1, AllComponents, OperandModifier.Absolute), RegScalar(2, X));

            Assert.Contains("(bitcast<u32>(abs(bitcast<i32>(bitcast<u32>(r1.x)))) & bitcast<u32>(r2.x))", Emitted(asm).Single(s => s.StartsWith("r0.x", StringComparison.Ordinal)));
        }

        [Fact]
        public void IntegerNegate_OfUnsignedReadsWrapsAround()
        {
            var asm = Scalar1().Instruction(Opcode.And, 0, Temp(0, 1), Src(OperandType.Temp, 1, AllComponents, OperandModifier.Negate), RegScalar(2, X));

            Assert.Contains("(0u - bitcast<u32>(r1.x))", Emitted(asm).Single(s => s.StartsWith("r0.x", StringComparison.Ordinal)));
        }

        [Fact]
        public void FloatAbsoluteAndNegate_AreAppliedToTheSource()
        {
            var asm = Scalar1().Instruction(Opcode.Mov, 0, Temp(0, 1), Src(OperandType.Temp, 1, AllComponents, OperandModifier.AbsoluteNegate));

            Assert.Contains("r0.x = (-abs(r1.x));", Emitted(asm));
        }

        [Fact]
        public void Saturate_UnderStrictNanPolicy_ReplacesNanWithZero()
        {
            var asm = Scalar1().Instruction(Opcode.Mov, Saturate, Temp(0, 1), RegScalar(1, X));

            var statements = Emitted(asm, new WgslEmitOptions(NanPolicy.D3dStrict));

            Assert.Contains("let tmp0 = r1.x;", statements);
            Assert.Contains("r0.x = select(clamp(tmp0, 0.0, 1.0), 0.0, tmp0 != tmp0);", statements);
        }

        [Fact]
        public void Saturate_AppliesToTheOpcodeResultNotItsSources()
        {
            var asm = Scalar1().Instruction(Opcode.Mad, Saturate, Temp(0, 1), RegScalar(1, X), RegScalar(2, X), RegScalar(3, X));

            Assert.Contains("r0.x = clamp((r1.x * r2.x + r3.x), 0.0, 1.0);", Emitted(asm));
        }

        [Theory]
        [InlineData(1u, "bits_00000001")]
        [InlineData(0x007FFFFFu, "bits_007fffff")]
        public void DenormalLiteral_IsMaterialisedAtRuntimeSoFlushToZeroCannotChangeIt(uint bits, string variable)
        {
            var asm = Scalar1().Instruction(Opcode.Mov, 0, Temp(0, 1), ImmBits(bits, 0, 0, 0));

            var statements = Emitted(asm);

            Assert.Contains($"var {variable}: u32 = 0x{bits:x8}u;", statements);
            Assert.Contains($"r0.x = bitcast<f32>({variable});", statements);
        }

        [Fact]
        public void ZeroAndNormalLiterals_StayInline()
        {
            var asm = Scalar1()
                .Instruction(Opcode.Mov, 0, Temp(0, 1), Imm(0.5f, 0f, 0f, 0f))
                .Instruction(Opcode.Mov, 0, Temp(1, 1), ImmBits(0x80000000u, 0, 0, 0));

            var statements = Emitted(asm);

            Assert.Contains("r0.x = 0.5;", statements);
            Assert.Contains("r1.x = (-0.0);", statements);
            Assert.DoesNotContain(statements, s => s.StartsWith("var bits_", StringComparison.Ordinal));
        }

        [Fact]
        public void MinMax_MapToTheirOwnBuiltins()
        {
            var asm = Scalar1()
                .Instruction(Opcode.Min, 0, Temp(0, 1), RegScalar(1, X), RegScalar(2, X))
                .Instruction(Opcode.Max, 0, Temp(1, 1), RegScalar(1, X), RegScalar(2, X));

            var statements = Emitted(asm);

            Assert.Contains("r0.x = min(r1.x, r2.x);", statements);
            Assert.Contains("r1.x = max(r1.x, r2.x);", statements);
        }

        [Fact]
        public void Swizzles_AreCompactedToTheDestinationWidth()
        {
            var asm = Scalar1().Instruction(Opcode.Add, 0, Temp(0, 0b0101), Reg(1, Swz(W, X, Z, Y)), Reg(2, Swz(Y, Y, X, X)));

            var statements = Emitted(asm);

            Assert.Contains("let tmp0 = (r1.wz + r2.yx);", statements);
        }

        [Theory]
        [InlineData(Opcode.AtomicUmax, "atomicMax")]
        [InlineData(Opcode.AtomicUmin, "atomicMin")]
        [InlineData(Opcode.AtomicIadd, "atomicAdd")]
        [InlineData(Opcode.AtomicAnd, "atomicAnd")]
        [InlineData(Opcode.AtomicOr, "atomicOr")]
        [InlineData(Opcode.AtomicXor, "atomicXor")]
        public void Atomics_MapToTheMatchingWgslFunction(Opcode opcode, string function)
        {
            var asm = Compute().Declaration(Opcode.DclThreadGroup, 0, [1u], [1u], [1u])
                .Declaration(Opcode.DclUavRaw, 0, [OperandToken(OperandType.UnorderedAccessView, 1, 0, 0, 0, [0]), 0u]).Temps(2)
                .Instruction(opcode, 0, Uav(0, 0xF), Bits(0), RegScalar(1, X))
                .Instruction(Opcode.Ret, 0);
            var map = new ShaderBindingMap();
            map.UnorderedAccess[0] = new ResourceBinding("blob_u0", 0, 0);

            string wgsl = Emit(asm, map).Wgsl;

            Assert.Contains($"_ = {function}(&blob_u0[(0u >> 2u)], bitcast<u32>(r1.x));", wgsl);
        }
    }
}
