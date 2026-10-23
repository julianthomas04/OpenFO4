using AssetConverter.ShaderTranslation;
using AssetConverter.ShaderTranslation.Dxbc;
using AssetConverter.ShaderTranslation.Wgsl;
using InstallerTests.Support;
using Xunit;
using static InstallerTests.Support.Sm5Assembler;

namespace InstallerTests.ShaderTranslation
{
    public class WgslComputeTests
    {
        private const uint SyncGroupAndShared = 0x3;
        private const uint SyncGlobalUav = 0x8;
        private const uint SyncGroupUav = 0x5;

        private static Sm5Assembler Compute() => new(ShaderStage.Compute);

        private static uint[] ThreadIdDst(OperandType type, int mask) => [OperandToken(type, 0, 2, 0, mask, [])];

        private static uint[] ThreadIdSrc(OperandType type, uint[] swizzle) => [OperandToken(type, 0, 2, 1, (int)swizzle[0], [])];

        private static uint[] ThreadGroupMemory(int register, int mask) => [OperandToken(OperandType.ThreadGroupSharedMemory, 1, 2, 0, mask, [0]), (uint)register];

        private static uint[] ThreadGroupMemorySource(int register, uint[] swizzle) =>
            [OperandToken(OperandType.ThreadGroupSharedMemory, 1, 2, 1, (int)swizzle[0], [0]), (uint)register];

        private static uint[] Uav(int register, int mask) => [OperandToken(OperandType.UnorderedAccessView, 1, 2, 0, mask, [0]), (uint)register];

        private static Sm5Assembler WithThreadGroup(Sm5Assembler asm, uint x = 8, uint y = 8, uint z = 1) =>
            asm.Declaration(Opcode.DclThreadGroup, 0, [x], [y], [z]);

        private static Sm5Assembler WithSharedMemory(Sm5Assembler asm, int register, uint stride, uint count) =>
            asm.Declaration(Opcode.DclTgsmStructured, 0, [OperandToken(OperandType.ThreadGroupSharedMemory, 1, 0, 0, 0, [0]), (uint)register], [stride], [count]);

        private static Sm5Assembler WithTypedUav(Sm5Assembler asm, int register, ResourceDimension dimension = ResourceDimension.Texture2D) =>
            asm.Declaration(Opcode.DclUavTyped, (uint)dimension, [OperandToken(OperandType.UnorderedAccessView, 1, 0, 0, 0, [0]), (uint)register], [0x5555u]);

        private static EmittedShader Emit(Sm5Assembler asm, ShaderBindingMap? map = null) =>
            WgslTranslator.Emit("cs", Sm5Decoder.Decode(asm.BuildDxbc()), map);

        private static ShaderBindingMap UavMap(string? format, int register = 0)
        {
            var map = new ShaderBindingMap();
            map.UnorderedAccess[register] = new ResourceBinding("cs_u0", 0, 0, UavFormat: format);
            return map;
        }

        [Fact]
        public void Emit_ThreadGroupSizeAndThreadIds_AreExposedToTheWrapper()
        {
            var asm = WithThreadGroup(Compute(), 8, 4, 2).Declaration(Opcode.DclInput, 0, ThreadIdDst(OperandType.InputThreadId, 0b0011)).Temps(1)
                .Instruction(Opcode.Mov, 0, Dst(OperandType.Temp, 0, 0b0011), ThreadIdSrc(OperandType.InputThreadId, Swz(X, Y, Y, Y)))
                .Instruction(Opcode.Ret, 0);

            var shader = Emit(asm);

            Assert.Equal((8u, 4u, 2u), shader.ThreadGroupSize);
            Assert.Contains("const cs_workgroup_size_x: u32 = 8u;", shader.Wgsl);
            Assert.Contains("const cs_workgroup_size_z: u32 = 2u;", shader.Wgsl);
            Assert.Contains("vThreadId: vec3<u32>,", shader.Wgsl);
            Assert.Contains("vec4<u32>(i.vThreadId, 0u).xy", shader.Wgsl);
            Assert.DoesNotContain("@compute", shader.Wgsl);
        }

        [Fact]
        public void Emit_ThreadGroupSharedMemoryWithBarrier_UsesWorkgroupVariableAndBarrier()
        {
            var asm = WithSharedMemory(WithThreadGroup(Compute()), 0, 4, 64).Temps(2)
                .Instruction(Opcode.StoreStructured, 0, ThreadGroupMemory(0, 0b0001), Scalar(OperandType.Temp, 0, X), ImmBits(0, 0, 0, 0), Scalar(OperandType.Temp, 1, X))
                .Instruction(Opcode.Sync, SyncGroupAndShared)
                .Instruction(Opcode.LdStructured, 0, Dst(OperandType.Temp, 1, 0b0001), Scalar(OperandType.Temp, 0, X), ImmBits(0, 0, 0, 0), ThreadGroupMemorySource(0, Swz(X, X, X, X)))
                .Instruction(Opcode.Ret, 0);

            string wgsl = Emit(asm).Wgsl;

            Assert.Contains("var<workgroup> cs_g0: array<u32, 64>;", wgsl);
            Assert.Contains("workgroupBarrier();", wgsl);
            Assert.Matches(@"cs_g0\[tmp\d+ \+ 0u\] = bitcast<u32>\(r1\.x\);", wgsl);
            Assert.Matches(@"cs_g0\[tmp\d+ \+ 0u\]", wgsl);
        }

        [Fact]
        public void Emit_UavFenceInSync_AddsStorageAndTextureBarriersForPresentResources()
        {
            var asm = WithTypedUav(WithThreadGroup(Compute()), 0).Instruction(Opcode.Sync, SyncGroupUav).Instruction(Opcode.Ret, 0);

            string wgsl = Emit(asm, UavMap("r32float")).Wgsl;

            Assert.Contains("workgroupBarrier();", wgsl);
            Assert.Contains("textureBarrier();", wgsl);
            Assert.DoesNotContain("storageBarrier();", wgsl);
        }

        [Fact]
        public void Emit_GlobalUavFence_IsRejectedBecauseWgslCannotExpressIt()
        {
            var asm = WithThreadGroup(Compute()).Instruction(Opcode.Sync, SyncGlobalUav).Instruction(Opcode.Ret, 0);

            var ex = Assert.Throws<WgslUnsupportedException>(() => Emit(asm));

            Assert.Contains("global UAV fence", ex.Message);
        }

        [Fact]
        public void Emit_TypedUavStore_DeclaresStorageTextureInConfiguredFormat()
        {
            var asm = WithTypedUav(WithThreadGroup(Compute()), 0).Temps(2)
                .Instruction(Opcode.StoreUavTyped, 0, Uav(0, 0xF), Src(OperandType.Temp, 0, Swz(X, Y, Y, Y)), Src(OperandType.Temp, 1, Swz(Z, Z, Z, Z)))
                .Instruction(Opcode.Ret, 0);

            string wgsl = Emit(asm, UavMap("r32float")).Wgsl;

            Assert.Contains("@group(0) @binding(0) var cs_u0: texture_storage_2d<r32float, write>;", wgsl);
            Assert.Contains("textureStore(cs_u0, bitcast<vec2<u32>>(r0.xy), r1.zzzz);", wgsl);
        }

        [Fact]
        public void Emit_TypedUavLoadAndStore_UsesReadWriteAccessAndIntegerTexelsForUintFormats()
        {
            var asm = WithTypedUav(WithThreadGroup(Compute()), 0).Temps(2)
                .Instruction(Opcode.LdUavTyped, 0, Dst(OperandType.Temp, 1, 0b0001), Src(OperandType.Temp, 0, AllXyzw()), Uav(0, 0xF))
                .Instruction(Opcode.StoreUavTyped, 0, Uav(0, 0xF), Src(OperandType.Temp, 0, AllXyzw()), Src(OperandType.Temp, 1, AllXyzw()))
                .Instruction(Opcode.Ret, 0);

            string wgsl = Emit(asm, UavMap("r32uint")).Wgsl;

            Assert.Contains("texture_storage_2d<r32uint, read_write>", wgsl);
            Assert.Contains("textureLoad(cs_u0, bitcast<vec2<u32>>(r0.xy))", wgsl);
            Assert.Contains("textureStore(cs_u0, bitcast<vec2<u32>>(r0.xy), bitcast<vec4<u32>>(r1.xyzw));", wgsl);
        }

        [Fact]
        public void Emit_TypedUavWithoutFormat_ThrowsUnmappedResource()
        {
            var asm = WithTypedUav(WithThreadGroup(Compute()), 0).Instruction(Opcode.Ret, 0);

            Assert.Throws<UnmappedResourceException>(() => Emit(asm, UavMap(null)));
        }

        [Fact]
        public void Emit_TypedUavWithUnsupportedFormat_Throws()
        {
            var asm = WithTypedUav(WithThreadGroup(Compute()), 0).Instruction(Opcode.Ret, 0);

            Assert.Throws<WgslUnsupportedException>(() => Emit(asm, UavMap("r11g11b10float")));
        }

        [Fact]
        public void Emit_SharedMemoryAtomicAdd_DeclaresAtomicArrayAndReturnsOldValue()
        {
            var asm = Compute().Declaration(Opcode.DclThreadGroup, 0, [1u], [1u], [1u])
                .Declaration(Opcode.DclTgsmRaw, 0, [OperandToken(OperandType.ThreadGroupSharedMemory, 1, 0, 0, 0, [0]), 0u], [4u]).Temps(1)
                .Instruction(Opcode.ImmAtomicIadd, 0, Dst(OperandType.Temp, 0, 0b0001), ThreadGroupMemory(0, 0xF), ImmBits(0, 0, 0, 0), ImmBits(1, 1, 1, 1))
                .Instruction(Opcode.Ret, 0);

            string wgsl = Emit(asm).Wgsl;

            Assert.Contains("var<workgroup> cs_g0: array<atomic<u32>, 1>;", wgsl);
            Assert.Contains("r0.x = bitcast<f32>(atomicAdd(&cs_g0[(0u >> 2u)], 1u));", wgsl);
        }

        [Fact]
        public void Emit_ThreadIdInPixelShader_Throws()
        {
            var asm = new Sm5Assembler(ShaderStage.Pixel).Declaration(Opcode.DclInput, 0, ThreadIdDst(OperandType.InputThreadId, 0b0001)).Instruction(Opcode.Ret, 0);

            Assert.Throws<WgslUnsupportedException>(() => Emit(asm));
        }

        [Fact]
        public void Emit_ThreadGroupDeclarationInVertexShader_Throws()
        {
            var asm = WithThreadGroup(new Sm5Assembler(ShaderStage.Vertex)).Instruction(Opcode.Ret, 0);

            Assert.Throws<WgslUnsupportedException>(() => Emit(asm));
        }

        private const uint SyncThreads = 0x1;
        private const uint SyncSharedFenceOnly = 0x2;

        [Fact]
        public void Emit_FenceOnlySyncAtFunctionLevel_BecomesABarrierBecauseAllThreadsReachIt()
        {
            var asm = WithThreadGroup(Compute()).Instruction(Opcode.Sync, SyncSharedFenceOnly).Instruction(Opcode.Ret, 0);

            Assert.Contains("workgroupBarrier();", Emit(asm).Wgsl);
        }

        [Fact]
        public void Emit_FenceOnlySyncInsideBranch_IsRejectedBecauseWgslBarriersNeedUniformControlFlow()
        {
            var asm = WithThreadGroup(Compute()).Temps(1)
                .Instruction(Opcode.If, 0x80, Scalar(OperandType.Temp, 0, X))
                .Instruction(Opcode.Sync, SyncSharedFenceOnly)
                .Instruction(Opcode.Endif, 0)
                .Instruction(Opcode.Ret, 0);

            var ex = Assert.Throws<WgslUnsupportedException>(() => Emit(asm));

            Assert.Contains("fence-only sync", ex.Message);
        }

        [Fact]
        public void Emit_FenceOnlySyncAfterConditionalReturn_IsRejectedBecauseSomeThreadsMayHaveLeft()
        {
            var asm = WithThreadGroup(Compute()).Temps(1)
                .Instruction(Opcode.Retc, 0x80, Scalar(OperandType.Temp, 0, X))
                .Instruction(Opcode.Sync, SyncSharedFenceOnly)
                .Instruction(Opcode.Ret, 0);

            Assert.Throws<WgslUnsupportedException>(() => Emit(asm));
        }

        [Fact]
        public void Emit_FenceOnlySyncInsideLoop_IsRejected()
        {
            var asm = WithThreadGroup(Compute())
                .Instruction(Opcode.Loop, 0)
                .Instruction(Opcode.Sync, SyncSharedFenceOnly)
                .Instruction(Opcode.Break, 0)
                .Instruction(Opcode.Endloop, 0)
                .Instruction(Opcode.Ret, 0);

            Assert.Throws<WgslUnsupportedException>(() => Emit(asm));
        }

        [Fact]
        public void Emit_ThreadSyncInsideBranch_IsKeptBecauseD3dAlreadyDemandsUniformFlow()
        {
            var asm = WithThreadGroup(Compute()).Temps(1)
                .Instruction(Opcode.If, 0x80, Scalar(OperandType.Temp, 0, X))
                .Instruction(Opcode.Sync, SyncThreads | SyncSharedFenceOnly)
                .Instruction(Opcode.Endif, 0)
                .Instruction(Opcode.Ret, 0);

            Assert.Contains("        workgroupBarrier();", Emit(asm).Wgsl);
        }

        [Fact]
        public void Emit_SyncWithNoFlags_EmitsNothing()
        {
            var asm = WithThreadGroup(Compute()).Instruction(Opcode.Sync, 0).Instruction(Opcode.Ret, 0);

            Assert.DoesNotContain("Barrier", Emit(asm).Wgsl);
        }

        [Fact]
        public void Emit_UavFenceWithBufferUav_AddsStorageBarrier()
        {
            var asm = WithThreadGroup(Compute())
                .Declaration(Opcode.DclUavRaw, 0, [OperandToken(OperandType.UnorderedAccessView, 1, 0, 0, 0, [0]), 0u])
                .Instruction(Opcode.Sync, SyncGroupUav).Instruction(Opcode.Ret, 0);

            string wgsl = Emit(asm).Wgsl;

            Assert.Contains("storageBarrier();", wgsl);
            Assert.DoesNotContain("textureBarrier();", wgsl);
        }

        private static uint[] AllXyzw() => Swz(X, Y, Z, W);
    }
}
