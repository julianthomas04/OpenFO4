using AssetConverter.ShaderTranslation;
using AssetConverter.ShaderTranslation.Dxbc;
using InstallerTests.Support;
using Xunit;
using static InstallerTests.ShaderTranslation.WgslTestKit;
using static InstallerTests.Support.Sm5Assembler;

namespace InstallerTests.ShaderTranslation
{
    public class WgslTextureTests
    {
        private const uint ResinfoFloat = 0;
        private const uint ResinfoReciprocal = 1;
        private const uint ResinfoUint = 2;

        private static Sm5Assembler WithTexture(ResourceDimension dimension) =>
            Pixel().ResourceDeclaration(0, dimension).SamplerDeclaration(0).Temps(3);

        [Fact]
        public void Ld_Texture2D_TakesTheMipLevelFromTheFourthAddressComponent()
        {
            var asm = WithTexture(ResourceDimension.Texture2D)
                .Instruction(Opcode.Ld, 0, Temp(0), Reg(1), Resource(0, AllComponents));

            string wgsl = Body(asm);

            Assert.Contains("textureLoad(blob_t0, bitcast<vec2<i32>>(r1.xy), bitcast<i32>(r1.w))", wgsl);
            Assert.DoesNotContain("r1.z", wgsl);
        }

        [Fact]
        public void Ld_Texture3D_UsesThreeCoordinatesAndTheFourthComponentAsMip()
        {
            var asm = WithTexture(ResourceDimension.Texture3D)
                .Instruction(Opcode.Ld, 0, Temp(0), Reg(1), Resource(0, AllComponents));

            Assert.Contains("textureLoad(blob_t0, bitcast<vec3<i32>>(r1.xyz), bitcast<i32>(r1.w))", Body(asm));
        }

        [Fact]
        public void Ld_Texture1D_StillTakesTheMipFromTheFourthComponent()
        {
            var asm = WithTexture(ResourceDimension.Texture1D)
                .Instruction(Opcode.Ld, 0, Temp(0), Reg(1), Resource(0, AllComponents));

            Assert.Contains("textureLoad(blob_t0, bitcast<i32>(r1.x), bitcast<i32>(r1.w))", Body(asm));
        }

        [Fact]
        public void Ld_Texture2DArray_ReadsLayerFromThirdAndMipFromFourthComponent()
        {
            var asm = WithTexture(ResourceDimension.Texture2DArray)
                .Instruction(Opcode.Ld, 0, Temp(0), Reg(1), Resource(0, AllComponents));

            Assert.Contains("textureLoad(blob_t0, bitcast<vec2<i32>>(r1.xy), bitcast<i32>(r1.z), bitcast<i32>(r1.w))", Body(asm));
        }

        [Fact]
        public void LdMs_ReadsTheSampleIndexFromItsOwnOperand()
        {
            var asm = WithTexture(ResourceDimension.Texture2DMs)
                .Instruction(Opcode.LdMs, 0, Temp(0), Reg(1), Resource(0, AllComponents), RegScalar(2, Y));

            Assert.Contains("textureLoad(blob_t0, bitcast<vec2<i32>>(r1.xy), bitcast<i32>(r2.y))", Body(asm));
        }

        [Fact]
        public void Ld_TexelOffset_IsAddedToTheCoordinates()
        {
            var asm = WithTexture(ResourceDimension.Texture2D)
                .InstructionWithExtensions(Opcode.Ld, 0, [OffsetExtension(1, -1, 0)], Temp(0), Reg(1), Resource(0, AllComponents));

            Assert.Contains("(bitcast<vec2<i32>>(r1.xy) + vec2<i32>(1, -1))", Body(asm));
        }

        [Fact]
        public void Ld_OnRawBuffer_IsRejectedAsNotATexture()
        {
            var asm = Pixel().Temps(2)
                .Declaration(Opcode.DclResourceRaw, 0, Resource(0, AllComponents))
                .Instruction(Opcode.Ld, 0, Temp(0), Reg(1), Resource(0, AllComponents));

            var ex = Assert.Throws<WgslUnsupportedException>(() => Body(asm));
            Assert.Contains("not a texture", ex.Message);
        }

        [Theory]
        [InlineData(ResourceDimension.Texture2D, "vec4<bool>(true, true, false, false)")]
        [InlineData(ResourceDimension.TextureCube, "vec4<bool>(true, true, false, false)")]
        [InlineData(ResourceDimension.Texture1D, "vec4<bool>(true, false, false, false)")]
        [InlineData(ResourceDimension.Texture3D, "vec4<bool>(true, true, true, false)")]
        [InlineData(ResourceDimension.Texture2DArray, "vec4<bool>(true, true, false, false)")]
        public void Resinfo_ReciprocalReturnType_InvertsOnlyTheDimensionLanes(ResourceDimension dimension, string expectedMask)
        {
            var asm = WithTexture(dimension)
                .Instruction(Opcode.Resinfo, ResinfoReciprocal, Temp(0), RegScalar(1, X), Resource(0, AllComponents));

            string wgsl = Body(asm);

            Assert.Contains($"select(vec4<f32>(tmp0), vec4<f32>(1.0) / vec4<f32>(tmp0), {expectedMask})", wgsl);
        }

        [Fact]
        public void Resinfo_FloatReturnType_ConvertsAllLanesWithoutInversion()
        {
            var asm = WithTexture(ResourceDimension.Texture2D)
                .Instruction(Opcode.Resinfo, ResinfoFloat, Temp(0), RegScalar(1, X), Resource(0, AllComponents));

            string wgsl = Body(asm);

            Assert.Contains("r0 = vec4<f32>(tmp0).xyzw;", wgsl);
            Assert.DoesNotContain("1.0) /", wgsl);
        }

        [Fact]
        public void Resinfo_UintReturnType_PacksDimensionsLayersAndLevels()
        {
            var asm = WithTexture(ResourceDimension.Texture2DArray)
                .Instruction(Opcode.Resinfo, ResinfoUint, Temp(0), RegScalar(1, X), Resource(0, AllComponents));

            string wgsl = Body(asm);

            Assert.Contains("let tmp0 = vec4<u32>(textureDimensions(blob_t0, bitcast<u32>(r1.x)), textureNumLayers(blob_t0), textureNumLevels(blob_t0));", wgsl);
            Assert.Contains("r0 = bitcast<vec4<f32>>(tmp0.xyzw);", wgsl);
        }

        [Fact]
        public void SampleLevel_UsesTheLevelOperand()
        {
            var asm = WithTexture(ResourceDimension.Texture2D)
                .Instruction(Opcode.SampleL, 0, Temp(0), Reg(1), Resource(0, AllComponents), Sampler(0), RegScalar(2, Z));

            Assert.Contains("textureSampleLevel(blob_t0, blob_s0, r1.xy, r2.z)", Body(asm));
        }

        [Fact]
        public void SampleBias_UsesTheBiasOperand()
        {
            var asm = WithTexture(ResourceDimension.Texture2D)
                .Instruction(Opcode.SampleB, 0, Temp(0), Reg(1), Resource(0, AllComponents), Sampler(0), RegScalar(2, W));

            Assert.Contains("textureSampleBias(blob_t0, blob_s0, r1.xy, r2.w)", Body(asm));
        }

        [Fact]
        public void SampleGradient_PassesBothDerivativeVectors()
        {
            var asm = WithTexture(ResourceDimension.Texture2D)
                .Instruction(Opcode.SampleD, 0, Temp(0), Reg(1), Resource(0, AllComponents), Sampler(0), Reg(2), Reg(2, Swz(Z, W, Z, W)));

            Assert.Contains("textureSampleGrad(blob_t0, blob_s0, r1.xy, r2.xy, r2.zw)", Body(asm));
        }

        [Fact]
        public void Sample_TexelOffset_IsPassedAsTrailingArgument()
        {
            var asm = WithTexture(ResourceDimension.Texture2D)
                .InstructionWithExtensions(Opcode.Sample, 0, [OffsetExtension(2, -3, 0)],
                    Temp(0), Reg(1), Resource(0, AllComponents), Sampler(0));

            Assert.Contains("textureSample(blob_t0, blob_s0, r1.xy, vec2<i32>(2, -3))", Body(asm));
        }

        [Fact]
        public void Sample_Texture2DArray_RoundsTheLayerToNearest()
        {
            var asm = WithTexture(ResourceDimension.Texture2DArray)
                .Instruction(Opcode.Sample, 0, Temp(0), Reg(1), Resource(0, AllComponents), Sampler(0));

            Assert.Contains("textureSample(blob_t0, blob_s0, r1.xy, i32(floor(r1.z + 0.5)))", Body(asm));
        }

        [Fact]
        public void Gather4_UsesTheComponentSelectedOnTheSampler()
        {
            var asm = WithTexture(ResourceDimension.Texture2D)
                .Instruction(Opcode.Gather4, 0, Temp(0), Reg(1), Resource(0, AllComponents), Scalar(OperandType.Sampler, 0, Z));

            Assert.Contains("textureGather(2, blob_t0, blob_s0, r1.xy)", Body(asm));
        }

        [Fact]
        public void SampleOnDepthTexture_WidensTheScalarResultToAVector()
        {
            var asm = WithTexture(ResourceDimension.Texture2D)
                .Instruction(Opcode.SampleCLz, 0, Temp(0), Reg(1), Resource(0, AllComponents), Sampler(0), RegScalar(2, X))
                .Instruction(Opcode.SampleL, 0, Temp(1), Reg(1), Resource(0, AllComponents), Sampler(0), RegScalar(2, X));

            string wgsl = Body(asm);

            Assert.Contains("textureSampleCompareLevel(blob_t0, blob_s0, r1.xy, r2.x)", wgsl);
            Assert.Contains("vec4<f32>(textureSampleLevel(blob_t0, blob_s0, r1.xy, i32(r2.x)), 0.0, 0.0, 1.0)", wgsl);
        }

        [Theory]
        [InlineData(Opcode.SampleB, ResourceDimension.Texture1D)]
        [InlineData(Opcode.SampleD, ResourceDimension.Texture1D)]
        [InlineData(Opcode.Gather4, ResourceDimension.Texture3D)]
        [InlineData(Opcode.Gather4, ResourceDimension.Texture1D)]
        public void Sampling_FormsWithoutAWgslBuiltin_AreRejectedAtTranslationTime(Opcode opcode, ResourceDimension dimension)
        {
            uint[][] extra = opcode switch
            {
                Opcode.SampleB => [RegScalar(2, X)],
                Opcode.SampleD => [Reg(2), Reg(2)],
                _ => [],
            };
            var asm = WithTexture(dimension)
                .Instruction(opcode, 0, [Temp(0), Reg(1), Resource(0, AllComponents), Sampler(0), .. extra]);

            var ex = Assert.Throws<WgslUnsupportedException>(() => Body(asm));
            Assert.Contains(OpcodeInfo.Name(opcode), ex.Message);
        }

        [Fact]
        public void SampleGradient_OnDepthTexture_IsRejected()
        {
            var asm = WithTexture(ResourceDimension.Texture2D)
                .Instruction(Opcode.SampleC, 0, Temp(0), Reg(1), Resource(0, AllComponents), Sampler(0), RegScalar(2, X))
                .Instruction(Opcode.SampleD, 0, Temp(1), Reg(1), Resource(0, AllComponents), Sampler(0), Reg(2), Reg(2));

            Assert.Throws<WgslUnsupportedException>(() => Body(asm));
        }

        [Fact]
        public void Sample_WithoutSamplerOperand_IsATypedError()
        {
            var asm = WithTexture(ResourceDimension.Texture2D)
                .Instruction(Opcode.Sample, 0, Temp(0), Reg(1), Resource(0, AllComponents), Reg(2));

            var ex = Assert.Throws<WgslUnsupportedException>(() => Body(asm));
            Assert.Contains("Sampler", ex.Message);
        }
    }
}
