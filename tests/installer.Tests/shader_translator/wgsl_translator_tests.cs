using AssetConverter.ShaderTranslation;
using AssetConverter.ShaderTranslation.Dxbc;
using AssetConverter.ShaderTranslation.Wgsl;
using InstallerTests.Support;
using Xunit;
using static InstallerTests.Support.Sm5Assembler;

namespace InstallerTests.ShaderTranslation
{
    public class WgslTranslatorTests
    {
        private static Sm5Assembler Pixel() => new(ShaderStage.Pixel);

        private static EmittedShader Emit(Sm5Assembler asm, ShaderBindingMap? map = null, WgslEmitOptions? options = null, string name = "blob") =>
            WgslTranslator.Emit(name, Sm5Decoder.Decode(asm.BuildDxbc()), map, options);

        private static string Body(Sm5Assembler asm, WgslEmitOptions? options = null) => Emit(asm, null, options).Wgsl;

        private static readonly uint[] AllComponents = Swz(X, Y, Z, W);

        [Fact]
        public void Emit_ModuleScopeNamesArePrefixedWithShaderName()
        {
            var asm = Pixel().ConstantBufferDeclaration(1, 3).SamplerDeclaration(0).ResourceDeclaration(0, ResourceDimension.Texture2D)
                .OutputDeclaration(0, 0xF).Temps(1)
                .Instruction(Opcode.Mov, 0, Dst(OperandType.Output, 0, 0xF), ConstantBuffer(1, 2, AllComponents))
                .Instruction(Opcode.Ret, 0);

            var shader = Emit(asm, name: "water_ps");

            Assert.Contains("struct water_ps_in {", shader.Wgsl);
            Assert.Contains("struct water_ps_out {", shader.Wgsl);
            Assert.Contains("fn water_ps(i: water_ps_in) -> water_ps_out {", shader.Wgsl);
            Assert.All(shader.Bindings, b => Assert.StartsWith("water_ps_", b.Name));
            Assert.Contains("o.o0 = water_ps_cb1[2].xyzw;", shader.Wgsl);
        }

        [Fact]
        public void Emit_ExplicitBindingMap_DeclaresVariablesAtRequestedSlots()
        {
            var asm = Pixel().ConstantBufferDeclaration(1, 3).Temps(1)
                .Instruction(Opcode.Mov, 0, Dst(OperandType.Temp, 0, 0xF), ConstantBuffer(1, 0, AllComponents));
            var map = new ShaderBindingMap();
            map.ConstantBuffers[1] = new ResourceBinding("water_frame", 2, 5);

            var shader = Emit(asm, map);

            Assert.Contains("@group(2) @binding(5) var<uniform> water_frame: array<vec4<f32>, 3>;", shader.Wgsl);
            Assert.Contains("r0 = water_frame[0].xyzw;", shader.Wgsl);
            Assert.Equal(new EmittedBinding("cb", 1, "water_frame", 2, 5, "array<vec4<f32>, 3>", true), Assert.Single(shader.Bindings));
        }

        [Fact]
        public void Emit_ExternalBinding_IsReferencedButNotDeclared()
        {
            var asm = Pixel().ConstantBufferDeclaration(0, 1).Temps(1)
                .Instruction(Opcode.Mov, 0, Dst(OperandType.Temp, 0, 0xF), ConstantBuffer(0, 0, AllComponents));
            var map = new ShaderBindingMap();
            map.ConstantBuffers[0] = new ResourceBinding("shared_params", 0, 0, External: true);

            var shader = Emit(asm, map);

            Assert.DoesNotContain("var<uniform> shared_params", shader.Wgsl);
            Assert.Contains("shared_params[0]", shader.Wgsl);
            Assert.False(Assert.Single(shader.Bindings).Declared);
        }

        [Fact]
        public void Emit_RegisterMissingFromBindingMap_Throws()
        {
            var asm = Pixel().ConstantBufferDeclaration(4, 1).Instruction(Opcode.Ret, 0);

            Assert.Throws<UnmappedResourceException>(() => Emit(asm, new ShaderBindingMap()));
        }

        [Fact]
        public void Emit_FullWidthMad_UsesVectorExpression()
        {
            var asm = Pixel().Temps(4)
                .Instruction(Opcode.Mad, 0, Dst(OperandType.Temp, 0, 0xF), Src(OperandType.Temp, 1, AllComponents),
                    Src(OperandType.Temp, 2, AllComponents), Src(OperandType.Temp, 3, AllComponents));

            Assert.Contains("r0 = (r1.xyzw * r2.xyzw + r3.xyzw);", Body(asm));
        }

        [Fact]
        public void Emit_PartialMask_ComputesBeforeAssigningSoSourceOverlapIsSafe()
        {
            var asm = Pixel().Temps(3)
                .Instruction(Opcode.Add, 0, Dst(OperandType.Temp, 0, 0b0110), Src(OperandType.Temp, 0, Swz(W, Z, Y, X)), Src(OperandType.Temp, 1, AllComponents));

            string wgsl = Body(asm);

            Assert.Contains("let tmp0 = (r0.zy + r1.yz);", wgsl);
            Assert.Contains("r0.y = tmp0.x;", wgsl);
            Assert.Contains("r0.z = tmp0.y;", wgsl);
        }

        [Fact]
        public void Emit_FloatComparison_StoresAllOnesOrZeroBitPattern()
        {
            var asm = Pixel().Temps(3)
                .Instruction(Opcode.Lt, 0, Dst(OperandType.Temp, 0, 0b0001), Scalar(OperandType.Temp, 1, X), Scalar(OperandType.Temp, 2, X));

            Assert.Contains("r0.x = bitcast<f32>(select(0u, 0xffffffffu, (r1.x < r2.x)));", Body(asm));
        }

        [Fact]
        public void Emit_IntegerAdd_ReinterpretsRegistersAsSignedIntegers()
        {
            var asm = Pixel().Temps(2)
                .Instruction(Opcode.Iadd, 0, Dst(OperandType.Temp, 0, 0b0001), Scalar(OperandType.Temp, 1, Y), ImmBits(0xFFFFFFFFu, 0, 0, 0));

            Assert.Contains("r0.x = bitcast<f32>((bitcast<i32>(r1.y) + (-1i)));", Body(asm));
        }

        [Fact]
        public void Emit_UnsignedDivision_FollowsD3dDivideByZeroResult()
        {
            var asm = Pixel().Temps(3)
                .Instruction(Opcode.Udiv, 0, Dst(OperandType.Temp, 0, 0b0001), NullOperand(), Scalar(OperandType.Temp, 1, X), Scalar(OperandType.Temp, 2, X));

            string wgsl = Body(asm);

            Assert.Contains("0xffffffffu", wgsl);
            Assert.Contains("== 0u)", wgsl);
        }

        [Fact]
        public void Emit_Saturate_ClampsResult()
        {
            var asm = Pixel().Temps(2)
                .Instruction(Opcode.Mov, 0x4, Dst(OperandType.Temp, 0, 0b0001), Scalar(OperandType.Temp, 1, X));

            Assert.Contains("r0.x = clamp(r1.x, 0.0, 1.0);", Body(asm));
        }

        [Fact]
        public void Emit_NonFiniteImmediate_IsHiddenBehindRuntimeVariable()
        {
            var asm = Pixel().Temps(1)
                .Instruction(Opcode.Mov, 0, Dst(OperandType.Temp, 0, 0b0001), ImmBits(0x7FC00000u, 0, 0, 0));

            string wgsl = Body(asm);

            Assert.Contains("var bits_7fc00000: u32 = 0x7fc00000u;", wgsl);
            Assert.Contains("r0.x = bitcast<f32>(bits_7fc00000);", wgsl);
        }

        [Fact]
        public void Emit_MinMax_PolicyChangesNanHandling()
        {
            var asm = Pixel().Temps(3)
                .Instruction(Opcode.Min, 0, Dst(OperandType.Temp, 0, 0b0001), Scalar(OperandType.Temp, 1, X), Scalar(OperandType.Temp, 2, X));

            string plain = Body(asm);
            string strict = Body(asm, new WgslEmitOptions(NanPolicy.D3dStrict));

            Assert.Contains("r0.x = min(r1.x, r2.x);", plain);
            Assert.DoesNotContain("!=", plain);
            Assert.Contains("let tmp0 = r1.x;", strict);
            Assert.Contains("select(select(min(tmp0, tmp1), tmp0, tmp1 != tmp1), tmp1, tmp0 != tmp0)", strict);
        }

        [Fact]
        public void Emit_ControlFlow_ProducesStructuredWgsl()
        {
            var asm = Pixel().Temps(1)
                .Instruction(Opcode.Loop, 0)
                .Instruction(Opcode.Breakc, 0x80, Scalar(OperandType.Temp, 0, X))
                .Instruction(Opcode.Endloop, 0)
                .Instruction(Opcode.If, 0, Scalar(OperandType.Temp, 0, Y))
                .Instruction(Opcode.Discard, 0x80, Scalar(OperandType.Temp, 0, Z))
                .Instruction(Opcode.Endif, 0)
                .Instruction(Opcode.Ret, 0);

            string wgsl = Body(asm);

            Assert.Contains("    loop {\n        if (bitcast<u32>(r0.x) != 0u) {\n            break;\n        }\n    }\n", wgsl);
            Assert.Contains("if (bitcast<u32>(r0.y) == 0u) {", wgsl);
            Assert.Contains("discard_pending = true;", wgsl);
            Assert.EndsWith("    if (discard_pending) { discard; }\n    return o;\n}\n", wgsl);
        }

        [Fact]
        public void Emit_AdjacentCaseLabels_AreMergedAndDefaultIsAdded()
        {
            var asm = Pixel().Temps(1)
                .Instruction(Opcode.Switch, 0, Scalar(OperandType.Temp, 0, X))
                .Instruction(Opcode.Case, 0, ImmBits(1, 1, 1, 1))
                .Instruction(Opcode.Case, 0, ImmBits(2, 2, 2, 2))
                .Instruction(Opcode.Mov, 0, Dst(OperandType.Temp, 0, 0b0010), Imm(1f, 1f, 1f, 1f))
                .Instruction(Opcode.Break, 0)
                .Instruction(Opcode.Endswitch, 0);

            string wgsl = Body(asm);

            Assert.Contains("case 1, 2: {", wgsl);
            Assert.Contains("default: { }", wgsl);
        }

        [Fact]
        public void Emit_SwitchClauseFallingThrough_IsRejected()
        {
            var asm = Pixel().Temps(1)
                .Instruction(Opcode.Switch, 0, Scalar(OperandType.Temp, 0, X))
                .Instruction(Opcode.Case, 0, ImmBits(1, 1, 1, 1))
                .Instruction(Opcode.Mov, 0, Dst(OperandType.Temp, 0, 0b0010), Imm(1f, 1f, 1f, 1f))
                .Instruction(Opcode.Case, 0, ImmBits(2, 2, 2, 2))
                .Instruction(Opcode.Break, 0)
                .Instruction(Opcode.Endswitch, 0);

            Assert.Throws<WgslUnsupportedException>(() => Body(asm));
        }

        [Fact]
        public void Emit_TextureSample_SplitsTextureAndSampler()
        {
            var asm = Pixel().SamplerDeclaration(0).ResourceDeclaration(0, ResourceDimension.Texture2D).Temps(2)
                .Instruction(Opcode.Sample, 0, Dst(OperandType.Temp, 0, 0xF), Src(OperandType.Temp, 1, Swz(X, Y, X, X)),
                    Resource(0, AllComponents), Sampler(0));

            var shader = Emit(asm);

            Assert.Contains("let tmp0 = textureSample(blob_t0, blob_s0, r1.xy);", shader.Wgsl);
            Assert.Contains("r0 = tmp0.xyzw;", shader.Wgsl);
            Assert.Contains("@group(0) @binding(0) var blob_s0: sampler;", shader.Wgsl);
            Assert.Contains("@group(0) @binding(1) var blob_t0: texture_2d<f32>;", shader.Wgsl);
        }

        [Fact]
        public void Emit_ResourceSwizzle_SelectsFetchedComponents()
        {
            var asm = Pixel().SamplerDeclaration(0).ResourceDeclaration(0, ResourceDimension.Texture2D).Temps(2)
                .Instruction(Opcode.SampleL, 0, Dst(OperandType.Temp, 0, 0b0011), Src(OperandType.Temp, 1, Swz(X, Y, X, X)),
                    Resource(0, Swz(W, X, X, X)), Sampler(0), Imm(0f, 0f, 0f, 0f));

            string wgsl = Body(asm);

            Assert.Contains("textureSampleLevel(blob_t0, blob_s0, r1.xy, 0.0)", wgsl);
            Assert.Contains("let tmp1 = tmp0.wx;", wgsl);
        }

        [Fact]
        public void Emit_ComparisonSampling_DeclaresDepthTextureAndComparisonSampler()
        {
            var asm = Pixel().SamplerDeclaration(0, mode: 1).ResourceDeclaration(0, ResourceDimension.Texture2D).Temps(2)
                .Instruction(Opcode.SampleCLz, 0, Dst(OperandType.Temp, 0, 0b0001), Src(OperandType.Temp, 1, Swz(X, Y, X, X)),
                    Resource(0, Swz(X, X, X, X)), Sampler(0), Scalar(OperandType.Temp, 1, Z));

            string wgsl = Body(asm);

            Assert.Contains("var blob_t0: texture_depth_2d;", wgsl);
            Assert.Contains("var blob_s0: sampler_comparison;", wgsl);
            Assert.Contains("textureSampleCompareLevel(blob_t0, blob_s0, r1.xy, r1.z)", wgsl);
        }

        [Fact]
        public void Emit_TextureOverride_ForcesDepthTypeWithoutCompareUse()
        {
            var asm = Pixel().SamplerDeclaration(0).ResourceDeclaration(0, ResourceDimension.Texture2D).Instruction(Opcode.Ret, 0);
            var map = ShaderBindingMap.Sequential(Sm5Decoder.Decode(asm.BuildDxbc()), "blob");
            map.Resources[0] = map.Resources[0] with { TextureOverride = TextureOverride.Depth };

            Assert.Contains("var blob_t0: texture_depth_2d;", Emit(asm, map).Wgsl);
        }

        [Fact]
        public void Emit_RelativeConstantBufferIndex_UsesUnsignedRegisterBits()
        {
            var asm = Pixel().ConstantBufferDeclaration(1, 8).Temps(3)
                .Instruction(Opcode.Mov, 0, Dst(OperandType.Temp, 0, 0xF),
                    ConstantBufferRelative(1, Scalar(OperandType.Temp, 2, X), 3, AllComponents));

            Assert.Contains("r0 = blob_cb1[(bitcast<u32>(r2.x) + 3u)].xyzw;", Body(asm));
        }

        [Fact]
        public void Emit_InterfaceStructs_ListDeclaredRegistersWithSemantics()
        {
            var asm = new Sm5Assembler(ShaderStage.Vertex)
                .InputDeclaration(0, 0b0111).OutputDeclaration(1, 0xF).Temps(1)
                .Instruction(Opcode.Mov, 0, Dst(OperandType.Output, 1, 0xF), Src(OperandType.Input, 0, AllComponents))
                .Instruction(Opcode.Ret, 0)
                .Chunk("ISGN", Signature(("POSITION", 0, 0, 0, 0x7)));

            var shader = Emit(asm);

            var input = Assert.Single(shader.Inputs);
            Assert.Equal(("v0", "POSITION0", (byte)0x7), (input.Field, input.Semantic, input.Mask));
            Assert.Equal("o1", Assert.Single(shader.Outputs).Field);
            Assert.Contains("v0: vec4<f32>, // POSITION0 mask=0x7", shader.Wgsl);
            Assert.Contains("o.o1 = i.v0.xyzw;", shader.Wgsl);
        }

        [Fact]
        public void Emit_DepthOutput_AddsScalarFieldToOutputStruct()
        {
            var asm = Pixel().Temps(1)
                .Declaration(Opcode.DclOutput, 0, [OperandToken(OperandType.OutputDepth, 0, 1, 0, 0, [])])
                .Instruction(Opcode.Mov, 0, [OperandToken(OperandType.OutputDepth, 0, 1, 0, 0, [])], Scalar(OperandType.Temp, 0, X));

            string wgsl = Body(asm);

            Assert.Contains("oDepth: f32,", wgsl);
            Assert.Contains("o.oDepth = r0.x;", wgsl);
        }

        [Fact]
        public void Emit_StructuredBufferLoad_ReadsWordsFromStorageArray()
        {
            var asm = Pixel().Temps(2)
                .Declaration(Opcode.DclResourceStructured, 0, Resource(0, AllComponents), [32u])
                .Instruction(Opcode.LdStructured, 0, Dst(OperandType.Temp, 0, 0b0011), Scalar(OperandType.Temp, 1, X), ImmBits(8, 8, 8, 8), Resource(0, Swz(X, Y, Z, W)));

            string wgsl = Body(asm);

            Assert.Contains("var<storage, read> blob_t0: array<u32>;", wgsl);
            Assert.Contains("blob_t0[tmp0 + 0u]", wgsl);
            Assert.Contains("blob_t0[tmp0 + 1u]", wgsl);
            Assert.Contains("* 8u + (8u >> 2u))", wgsl);
        }

        [Fact]
        public void Emit_ImmediateConstantBuffer_BecomesPrivateArray()
        {
            var asm = Pixel().Temps(1).RawWords(53u | 3u << 11, 10, 0x3f800000, 0, 0, 0, 0x40000000, 0, 0, 0)
                .Instruction(Opcode.Ret, 0);

            Assert.Contains("var<private> blob_icb: array<vec4<u32>, 2> = array<vec4<u32>, 2>(vec4<u32>(0x3f800000u, 0x00000000u, 0x00000000u, 0x00000000u), vec4<u32>(0x40000000u", Body(asm));
        }

        [Theory]
        [InlineData(Opcode.Lod)]
        [InlineData(Opcode.Sampleinfo)]
        [InlineData(Opcode.Gather4Po)]
        public void Emit_UnsupportedOpcode_ThrowsTypedError(Opcode opcode)
        {
            var asm = Pixel().Temps(2).Instruction(opcode, 0, Dst(OperandType.Temp, 0, 0xF), Src(OperandType.Temp, 1, AllComponents));

            var ex = Assert.Throws<WgslUnsupportedException>(() => Body(asm));
            Assert.Contains(OpcodeInfo.Name(opcode), ex.Message);
        }

        [Fact]
        public void Emit_GeometryShader_IsRejected()
        {
            var asm = new Sm5Assembler(ShaderStage.Geometry).Instruction(Opcode.Ret, 0);

            Assert.Throws<WgslUnsupportedException>(() => Emit(asm));
        }

        [Fact]
        public void Emit_ProvenanceComment_IsPrependedToModule()
        {
            var shader = Emit(Pixel().Instruction(Opcode.Ret, 0), options: new WgslEmitOptions(ProvenanceComment: "blob 7"));

            Assert.StartsWith("// blob 7\n", shader.Wgsl.Replace("\r\n", "\n"));
        }
    }
}
