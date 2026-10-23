using AssetConverter.ShaderTranslation.Dxbc;
using AssetConverter.ShaderTranslation.Wgsl;
using InstallerTests.Support;
using Xunit;
using static InstallerTests.Support.Sm5Assembler;

namespace InstallerTests.ShaderTranslation
{
    /// <summary>Shared helpers of the emitter tests; every emission is linted so structural mistakes fail any test that triggers them.</summary>
    internal static class WgslTestKit
    {
        public static readonly uint[] AllComponents = Swz(X, Y, Z, W);

        public static Sm5Assembler Pixel() => new(ShaderStage.Pixel);

        public static Sm5Assembler Compute() => new(ShaderStage.Compute);

        public static EmittedShader Emit(Sm5Assembler asm, ShaderBindingMap? map = null, WgslEmitOptions? options = null, string name = "blob")
        {
            var shader = WgslTranslator.Emit(name, Sm5Decoder.Decode(asm.BuildDxbc()), map, options);
            Assert.Empty(WgslLint.Problems(shader.Wgsl));
            return shader;
        }

        public static string Body(Sm5Assembler asm, WgslEmitOptions? options = null) => Emit(asm, null, options).Wgsl;

        /// <summary>The statements of the translated function, one per element, without indentation.</summary>
        public static string[] Statements(string wgsl)
        {
            var lines = wgsl.Replace("\r\n", "\n").Split('\n');
            int start = Array.FindIndex(lines, l => l.StartsWith("fn ", StringComparison.Ordinal));
            return lines.Skip(start + 1).Select(l => l.Trim()).Where(l => l.Length > 0).ToArray();
        }

        public static uint[] Temp(int register, int mask = 0xF) => Dst(OperandType.Temp, register, mask);

        public static uint[] Reg(int register, uint[]? swizzle = null) => Src(OperandType.Temp, register, swizzle ?? AllComponents);

        public static uint[] RegScalar(int register, int component) => Scalar(OperandType.Temp, register, component);

        public static uint[] Uav(int register, int mask) => [OperandToken(OperandType.UnorderedAccessView, 1, 2, 0, mask, [0]), (uint)register];

        public static uint[] UavSource(int register, uint[] swizzle) => [OperandToken(OperandType.UnorderedAccessView, 1, 2, 1, (int)swizzle[0], [0]), (uint)register];

        /// <summary>Extended opcode token carrying a texel offset (each component is a signed 4-bit value).</summary>
        public static uint OffsetExtension(int u, int v, int w) => 1u | (uint)(u & 0xF) << 9 | (uint)(v & 0xF) << 13 | (uint)(w & 0xF) << 17;
    }
}
