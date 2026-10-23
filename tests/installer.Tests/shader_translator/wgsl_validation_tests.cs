using System.Diagnostics;
using AssetConverter.ShaderTranslation.Dxbc;
using AssetConverter.ShaderTranslation.Wgsl;
using InstallerTests.Support;
using Xunit;
using static InstallerTests.ShaderTranslation.WgslTestKit;
using static InstallerTests.Support.Sm5Assembler;

namespace InstallerTests.ShaderTranslation
{
    public class WgslLintTests
    {
        [Fact]
        public void Problems_WellFormedFunction_ReportsNothing()
        {
            const string Wgsl = "fn f(i: f_in) -> f_out {\n    var o: f_out;\n    var r0: vec4<f32>;\n    let tmp0 = r0.x;\n    if (tmp0 > 0.0) {\n        r0.y = tmp0;\n    }\n    return o;\n}\n";

            Assert.Empty(WgslLint.Problems(Wgsl));
        }

        [Fact]
        public void Problems_UnbalancedDelimiters_AreReported()
        {
            Assert.NotEmpty(WgslLint.Problems("fn f() -> o {\n    var r0: vec4<f32>;\n    r0.x = (1.0;\n}\n"));
            Assert.NotEmpty(WgslLint.Problems("fn f() -> o {\n    var r0: vec4<f32>;\n"));
        }

        [Fact]
        public void Problems_TemporaryUsedBeforeDeclaration_IsReported()
        {
            var problems = WgslLint.Problems("fn f() -> o {\n    var r0: vec4<f32>;\n    r0.x = tmp0;\n    let tmp0 = 1.0;\n}\n");

            Assert.Contains(problems, p => p.Contains("tmp0", StringComparison.Ordinal));
        }

        [Fact]
        public void Problems_TemporaryUsedOutsideItsBlock_IsReported()
        {
            var problems = WgslLint.Problems("fn f() -> o {\n    var r0: vec4<f32>;\n    if (true) {\n        let tmp0 = 1.0;\n    }\n    r0.x = tmp0;\n}\n");

            Assert.Contains(problems, p => p.Contains("tmp0", StringComparison.Ordinal));
        }

        [Fact]
        public void Problems_UndeclaredRegister_IsReported()
        {
            Assert.NotEmpty(WgslLint.Problems("fn f() -> o {\n    var r0: vec4<f32>;\n    r1.x = 1.0;\n}\n"));
        }

        [Fact]
        public void Problems_ImmediateDiscard_IsReported()
        {
            var problems = WgslLint.Problems("fn f() -> o {\n    discard;\n}\n");

            Assert.Contains(problems, p => p.Contains("discard", StringComparison.Ordinal));
        }

        [Fact]
        public void Problems_RedeclarationInOneScope_IsReported()
        {
            Assert.NotEmpty(WgslLint.Problems("fn f() -> o {\n    let tmp0 = 1.0;\n    let tmp0 = 2.0;\n}\n"));
        }
    }

    /// <summary>
    /// Runs naga (the compiler the engine validates with) over representative emissions, including every construct that once produced
    /// invalid WGSL. The executable is a development tool located through OPENFO4_NAGA: locally the cases skip while it is unset, but
    /// on GitHub Actions an unset or unusable variable fails them, so CI can never silently drop the validation.
    /// </summary>
    public class NagaValidationTests
    {
        private static readonly TimeSpan NagaTimeout = TimeSpan.FromSeconds(60);

        /// <summary>The naga capabilities the engine validates with (startup.rs capabilities_of over the required device features); naga's own default grants every capability.</summary>
        private const string EngineCapabilities =
            "TEXTURE_AND_SAMPLER_BINDING_ARRAY,STORAGE_TEXTURE_16BIT_NORM_FORMATS,EARLY_DEPTH_TEST,MULTISAMPLED_SHADING,CUBE_ARRAY_TEXTURES,SHADER_FLOAT16_IN_FLOAT32,TEXTURE_AND_SAMPLER_BINDING_ARRAY_NON_UNIFORM_INDEXING,STORAGE_BUFFER_BINDING_ARRAY_NON_UNIFORM_INDEXING";

        private static Sm5Assembler Scalar() => Pixel().Temps(6);

        private static uint[] Bits(uint value) => ImmBits(value, value, value, value);

        private static readonly Dictionary<string, Func<Sm5Assembler>> Shaders = new()
        {
            [nameof(LiteralShiftBeyondWidth)] = LiteralShiftBeyondWidth,
            [nameof(LiteralDivisionByZero)] = LiteralDivisionByZero,
            [nameof(LiteralReciprocalOfZero)] = LiteralReciprocalOfZero,
            [nameof(LiteralIntegerOverflow)] = LiteralIntegerOverflow,
            [nameof(UnsignedDivisionByLiteralZero)] = UnsignedDivisionByLiteralZero,
            [nameof(DenormalAndNanLiterals)] = DenormalAndNanLiterals,
            [nameof(DiscardBeforeSample)] = DiscardBeforeSample,
            [nameof(TextureLoadAndResinfo)] = TextureLoadAndResinfo,
            [nameof(Bitfields)] = Bitfields,
            [nameof(StrictSaturate)] = StrictSaturate,
            [nameof(SharedMemoryWithBarrier)] = SharedMemoryWithBarrier,
            [nameof(HalfFloatConversions)] = HalfFloatConversions,
        };

        public static TheoryData<string> Cases() => [.. Shaders.Keys];

        private static Sm5Assembler LiteralShiftBeyondWidth() => Scalar()
            .Instruction(Opcode.Ishl, 0, Temp(0, 1), Bits(1), Bits(40))
            .Instruction(Opcode.Ushr, 0, Temp(1, 1), RegScalar(4, X), Bits(32))
            .Instruction(Opcode.Ishr, 0, Temp(2, 1), Bits(0xF0000000u), Bits(64));

        private static Sm5Assembler LiteralDivisionByZero() => Scalar().Instruction(Opcode.Div, 0, Temp(0, 1), Imm(1f, 1f, 1f, 1f), Imm(0f, 0f, 0f, 0f));

        private static Sm5Assembler LiteralReciprocalOfZero() => Scalar().Instruction(Opcode.Rcp, 0, Temp(0, 1), Imm(0f, 0f, 0f, 0f));

        private static Sm5Assembler LiteralIntegerOverflow() => Scalar()
            .Instruction(Opcode.Iadd, 0, Temp(0, 1), Bits(int.MaxValue), Bits(1))
            .Instruction(Opcode.Imad, 0, Temp(1, 1), Bits(int.MaxValue), Bits(4), Bits(1))
            .Instruction(Opcode.Mul, 0, Temp(2, 1), Imm(3e38f, 0, 0, 0), Imm(3e38f, 0, 0, 0));

        private static Sm5Assembler UnsignedDivisionByLiteralZero() => Scalar()
            .Instruction(Opcode.Udiv, 0, Temp(0, 1), Temp(1, 1), RegScalar(2, X), Bits(0))
            .Instruction(Opcode.Udiv, 0, Temp(2, 1), Temp(3, 1), Bits(7), Bits(0));

        private static Sm5Assembler DenormalAndNanLiterals() => Scalar()
            .Instruction(Opcode.Mov, 0, Temp(0, 1), ImmBits(1, 0, 0, 0))
            .Instruction(Opcode.Mov, 0, Temp(1, 1), ImmBits(0x7FC00000u, 0, 0, 0))
            .Instruction(Opcode.Mov, 0, Temp(2, 1), ImmBits(0x7F800000u, 0, 0, 0));

        private static Sm5Assembler DiscardBeforeSample() => Pixel().SamplerDeclaration(0).ResourceDeclaration(0, ResourceDimension.Texture2D).Temps(3)
            .Instruction(Opcode.Discard, 0x80, RegScalar(0, X))
            .Instruction(Opcode.Sample, 0, Temp(1), Reg(0), Resource(0, AllComponents), Sampler(0))
            .Instruction(Opcode.Retc, 0x80, RegScalar(0, Y))
            .Instruction(Opcode.Mov, 0, Temp(1), Reg(0))
            .Instruction(Opcode.Ret, 0);

        private static Sm5Assembler TextureLoadAndResinfo() => Pixel().SamplerDeclaration(0).ResourceDeclaration(0, ResourceDimension.Texture2DArray).Temps(3)
            .Instruction(Opcode.Ld, 0, Temp(0), Reg(1), Resource(0, AllComponents))
            .Instruction(Opcode.Resinfo, 1, Temp(1), RegScalar(1, X), Resource(0, AllComponents))
            .Instruction(Opcode.Resinfo, 2, Temp(2), RegScalar(1, X), Resource(0, AllComponents));

        private static Sm5Assembler Bitfields() => Scalar()
            .Instruction(Opcode.Ubfe, 0, Temp(0, 1), Bits(40), Bits(30), RegScalar(1, X))
            .Instruction(Opcode.Ibfe, 0, Temp(1, 1), RegScalar(1, X), RegScalar(2, X), RegScalar(3, X))
            .Instruction(Opcode.Bfi, 0, Temp(2, 1), Bits(8), Bits(28), RegScalar(3, X), RegScalar(4, X));

        private static Sm5Assembler StrictSaturate() => Scalar()
            .Instruction(Opcode.Mad, 0x4, Temp(0, 1), RegScalar(1, X), RegScalar(2, X), RegScalar(3, X))
            .Instruction(Opcode.Min, 0, Temp(1, 1), RegScalar(1, X), RegScalar(2, X));

        private static Sm5Assembler SharedMemoryWithBarrier() => Compute()
            .Declaration(Opcode.DclThreadGroup, 0, [8u], [1u], [1u])
            .Declaration(Opcode.DclTgsmStructured, 0, [OperandToken(OperandType.ThreadGroupSharedMemory, 1, 0, 0, 0, [0]), 0u], [4u], [8u]).Temps(2)
            .Instruction(Opcode.Sync, 0x2)
            .Instruction(Opcode.Sync, 0x3)
            .Instruction(Opcode.Ret, 0);

        private static Sm5Assembler HalfFloatConversions() => Scalar()
            .Instruction(Opcode.F32ToF16, 0, Temp(0, 1), RegScalar(1, X))
            .Instruction(Opcode.F16ToF32, 0, Temp(1), Reg(2));

        private static string Wrapper(Sm5Assembler asm, string name)
        {
            bool compute = Sm5Decoder.Decode(asm.BuildDxbc()).Stage == ShaderStage.Compute;
            return compute
                ? $"@compute @workgroup_size(1) fn entry() {{ var i: {name}_in; let o = {name}(i); }}"
                : $"@fragment fn entry() -> @location(0) vec4<f32> {{ var i: {name}_in; let o = {name}(i); return vec4<f32>(0.0); }}";
        }

        private static string RequireNagaExecutable()
        {
            switch (NagaLocator.FromEnvironment())
            {
                case NagaResolution.Run run:
                    return run.ExecutablePath;
                case NagaResolution.Skip skip:
                    Assert.Skip(skip.Reason);
                    break;
                case NagaResolution.Fail fail:
                    Assert.Fail(fail.Reason);
                    break;
            }

            throw new UnreachableException();
        }

        private static async Task<(int ExitCode, string Errors)> RunNagaAsync(string naga, string wgslFile)
        {
            var start = new ProcessStartInfo(naga, $"--capabilities {EngineCapabilities} \"{wgslFile}\"") { RedirectStandardError = true, RedirectStandardOutput = true, UseShellExecute = false };
            using var process = Process.Start(start)!;
            using var deadline = new CancellationTokenSource(NagaTimeout);
            try
            {
                var output = process.StandardOutput.ReadToEndAsync(deadline.Token);
                var errors = process.StandardError.ReadToEndAsync(deadline.Token);
                await process.WaitForExitAsync(deadline.Token);
                await output;
                return (process.ExitCode, await errors);
            }
            catch (OperationCanceledException)
            {
                process.Kill(entireProcessTree: true);
                await process.WaitForExitAsync();
                throw new TimeoutException($"naga ({naga}) did not finish within {NagaTimeout.TotalSeconds:0} s and was killed.");
            }
        }

        [Theory]
        [MemberData(nameof(Cases))]
        public async Task Emission_IsAcceptedByNaga(string caseName)
        {
            string naga = RequireNagaExecutable();

            var asm = Shaders[caseName]();
            var options = caseName == nameof(StrictSaturate) ? new WgslEmitOptions(NanPolicy.D3dStrict) : null;
            string module = Emit(asm, options: options, name: "blob").Wgsl + "\n" + Wrapper(asm, "blob");
            string file = Path.Combine(Path.GetTempPath(), $"naga-{Guid.NewGuid():N}.wgsl");
            File.WriteAllText(file, module);
            try
            {
                var (exitCode, errors) = await RunNagaAsync(naga, file);
                Assert.True(exitCode == 0, $"naga rejected {caseName}:\n{errors}\n{module}");
            }
            finally
            {
                File.Delete(file);
            }
        }
    }
}
