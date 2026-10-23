using AssetConverter.ShaderTranslation;
using AssetConverter.ShaderTranslation.Dxbc;
using AssetConverter.ShaderTranslation.Wgsl;
using InstallerTests.Support;
using Xunit;
using static InstallerTests.ShaderTranslation.WgslTestKit;
using static InstallerTests.Support.Sm5Assembler;

namespace InstallerTests.ShaderTranslation
{
    public class WgslBuffersAndLimitsTests
    {
        private static uint[] DeclaredResource(int register) => [OperandToken(OperandType.Resource, 1, 0, 0, 0, [0]), (uint)register];

        private static uint[] DeclaredUav(int register) => [OperandToken(OperandType.UnorderedAccessView, 1, 0, 0, 0, [0]), (uint)register];

        private static Sm5Assembler ComputeWithUav(Opcode declaration, params uint[][] extra) =>
            Compute().Declaration(Opcode.DclThreadGroup, 0, [1u], [1u], [1u]).Declaration(declaration, 0, [DeclaredUav(0), .. extra]).Temps(3);

        [Fact]
        public void BufferInfo_OfRawBuffer_ReturnsTheByteSize()
        {
            var asm = Pixel().Temps(1).Declaration(Opcode.DclResourceRaw, 0, DeclaredResource(0))
                .Instruction(Opcode.Bufinfo, 0, Temp(0, 1), Resource(0, AllComponents));

            Assert.Contains("r0.x = bitcast<f32>((arrayLength(&blob_t0) * 4u));", Statements(Body(asm)));
        }

        [Fact]
        public void BufferInfo_OfStructuredBuffer_ReturnsTheElementCount()
        {
            var asm = Pixel().Temps(1).Declaration(Opcode.DclResourceStructured, 0, DeclaredResource(0), [16u])
                .Instruction(Opcode.Bufinfo, 0, Temp(0, 1), Resource(0, AllComponents));

            Assert.Contains("r0.x = bitcast<f32>((arrayLength(&blob_t0) / 4u));", Statements(Body(asm)));
        }

        [Fact]
        public void StoreRaw_WritesWordsAtTheByteOffsetDividedByFour()
        {
            var asm = ComputeWithUav(Opcode.DclUavRaw)
                .Instruction(Opcode.StoreRaw, 0, Uav(0, 0b0011), RegScalar(1, X), Reg(2))
                .Instruction(Opcode.Ret, 0);

            var statements = Statements(Emit(asm).Wgsl);

            Assert.Contains("let tmp0 = (bitcast<u32>(r1.x) >> 2u);", statements);
            Assert.Contains("let tmp1 = bitcast<vec2<u32>>(r2.xy);", statements);
            Assert.Contains("blob_u0[tmp0 + 0u] = tmp1.x;", statements);
            Assert.Contains("blob_u0[tmp0 + 1u] = tmp1.y;", statements);
        }

        [Fact]
        public void LoadRaw_ReadsTheSelectedWordsOfTheResource()
        {
            var asm = Pixel().Temps(2).Declaration(Opcode.DclResourceRaw, 0, DeclaredResource(0))
                .Instruction(Opcode.LdRaw, 0, Temp(0, 0b0011), RegScalar(1, X), Resource(0, Swz(Z, W, X, X)));

            var statements = Statements(Body(asm));

            Assert.Contains("let tmp1 = bitcast<vec2<f32>>(vec2<u32>(blob_t0[tmp0 + 2u], blob_t0[tmp0 + 3u]));", statements);
            Assert.Contains("r0.x = tmp1.x;", statements);
            Assert.Contains("r0.y = tmp1.y;", statements);
        }

        [Fact]
        public void StructuredAtomic_UsesTheSameWordIndexAsStructuredLoads()
        {
            var asm = ComputeWithUav(Opcode.DclUavStructured, [8u])
                .Instruction(Opcode.AtomicIadd, 0, Uav(0, 0xF), Reg(1, Swz(X, Y, Y, Y)), RegScalar(2, X))
                .Instruction(Opcode.LdStructured, 0, Temp(0, 1), RegScalar(1, X), RegScalar(1, Y), UavSource(0, Swz(X, X, X, X)))
                .Instruction(Opcode.Ret, 0);

            var statements = Statements(Emit(asm).Wgsl);

            Assert.Contains("_ = atomicAdd(&blob_u0[(bitcast<u32>(r1.x) * 2u + (bitcast<u32>(r1.y) >> 2u))], bitcast<u32>(r2.x));", statements);
            Assert.Contains("let tmp0 = (bitcast<u32>(r1.x) * 2u + (bitcast<u32>(r1.y) >> 2u));", statements);
        }

        [Fact]
        public void RawBufferOperations_OnTypedUav_AreRejectedWithATypedError()
        {
            var asm = Compute().Declaration(Opcode.DclThreadGroup, 0, [1u], [1u], [1u])
                .Declaration(Opcode.DclUavTyped, (uint)ResourceDimension.Texture2D, DeclaredUav(0), [0x5555u]).Temps(3)
                .Instruction(Opcode.LdRaw, 0, Temp(0, 1), RegScalar(1, X), UavSource(0, Swz(X, X, X, X)))
                .Instruction(Opcode.Ret, 0);
            var map = new ShaderBindingMap();
            map.UnorderedAccess[0] = new ResourceBinding("blob_u0", 0, 0, UavFormat: "r32float");

            var ex = Assert.Throws<WgslUnsupportedException>(() => Emit(asm, map));

            Assert.Contains("typed UAV", ex.Message);
        }

        [Fact]
        public void Atomics_OnTypedUav_AreRejectedWithATypedError()
        {
            var asm = Compute().Declaration(Opcode.DclThreadGroup, 0, [1u], [1u], [1u])
                .Declaration(Opcode.DclUavTyped, (uint)ResourceDimension.Texture2D, DeclaredUav(0), [0x5555u]).Temps(3)
                .Instruction(Opcode.AtomicIadd, 0, Uav(0, 0xF), RegScalar(1, X), RegScalar(2, X))
                .Instruction(Opcode.Ret, 0);
            var map = new ShaderBindingMap();
            map.UnorderedAccess[0] = new ResourceBinding("blob_u0", 0, 0, UavFormat: "r32float");

            Assert.Throws<WgslUnsupportedException>(() => Emit(asm, map));
        }

        [Theory]
        [InlineData(4096u, true)]
        [InlineData(4097u, false)]
        public void TempRegisterCount_IsBoundedByTheD3dLimit(uint count, bool accepted)
        {
            var asm = Pixel().Declaration(Opcode.DclTemps, 0, [count]).Instruction(Opcode.Ret, 0);

            if (accepted)
                Assert.Contains("var r4095: vec4<f32>;", Body(asm));
            else
                Assert.Contains("dcl_temps", Assert.Throws<WgslUnsupportedException>(() => Body(asm)).Message);
        }

        [Theory]
        [InlineData(4096u, true)]
        [InlineData(4097u, false)]
        [InlineData(0u, false)]
        public void IndexableTempSize_IsBoundedByTheD3dLimit(uint size, bool accepted)
        {
            var asm = Pixel().Declaration(Opcode.DclIndexableTemp, 0, [0u], [size], [4u]).Instruction(Opcode.Ret, 0);

            if (accepted)
                Assert.Contains($"var x0: array<vec4<f32>, {size}>;", Body(asm));
            else
                Assert.Contains("dcl_indexableTemp", Assert.Throws<WgslUnsupportedException>(() => Body(asm)).Message);
        }

        [Fact]
        public void ConstantBuffer_OfUnknownSize_IsDeclaredAtTheMaximum()
        {
            var asm = Pixel().ConstantBufferDeclaration(0, 0).Instruction(Opcode.Ret, 0);

            Assert.Contains("var<uniform> blob_cb0: array<vec4<f32>, 4096>;", Body(asm));
        }

        [Fact]
        public void ConstantBuffer_LargerThanTheD3dLimit_IsRejected()
        {
            var asm = Pixel().ConstantBufferDeclaration(0, 4097).Instruction(Opcode.Ret, 0);

            Assert.Throws<WgslUnsupportedException>(() => Body(asm));
        }

        [Fact]
        public void MalformedDeclaration_SurfacesAsTypedErrorNamingTheInstruction()
        {
            var asm = Pixel().Declaration(Opcode.DclConstantBuffer, 0, Dst(OperandType.ConstantBuffer, 0, 0xF)).Instruction(Opcode.Ret, 0);

            var ex = Assert.Throws<WgslUnsupportedException>(() => Body(asm));

            Assert.Contains("dcl_constantBuffer", ex.Message);
            Assert.IsType<ArgumentOutOfRangeException>(ex.InnerException);
        }

        [Fact]
        public void DeclarationsOfOtherPipelines_AreRejectedInsteadOfIgnored()
        {
            var asm = Pixel().Declaration(Opcode.DclFunctionBody, 0, [0u]).Instruction(Opcode.Ret, 0);

            var ex = Assert.Throws<WgslUnsupportedException>(() => Body(asm));

            Assert.Contains("dcl_function_body", ex.Message);
        }

        [Fact]
        public void ResourceOperandOfTheWrongKind_IsATypedErrorNotInvalidWgsl()
        {
            var asm = Pixel().Temps(2).Declaration(Opcode.DclResourceRaw, 0, DeclaredResource(0))
                .Instruction(Opcode.Bufinfo, 0, Temp(0, 1), Reg(1));

            Assert.Throws<WgslUnsupportedException>(() => Body(asm));
        }
    }
}
