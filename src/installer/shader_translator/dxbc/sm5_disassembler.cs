using System.Globalization;
using System.Text;

namespace AssetConverter.ShaderTranslation.Dxbc
{
    /// <summary>
    /// Text rendering of the decoded program in the style of vkd3d's d3d-asm output (used for golden comparisons
    /// and diagnostics). Immediates are printed as raw bit patterns.
    /// </summary>
    public static class Sm5Disassembler
    {
        private static readonly string[] StageNames = ["ps", "vs", "gs", "hs", "ds", "cs"];
        private static readonly string[] DimensionNames =
        [
            "unknown", "buffer", "texture1d", "texture2d", "texture2dms", "texture3d", "texturecube", "texture1darray",
            "texture2darray", "texture2dmsarray", "texturecubearray", "raw_buffer", "structured_buffer",
        ];
        private static readonly string[] ReturnTypeNames = ["none", "unorm", "snorm", "int", "uint", "float", "mixed", "double", "continued", "unused"];
        private static readonly string[] SystemValueNames =
        [
            "undefined", "position", "clip_distance", "cull_distance", "render_target_array_index", "viewport_array_index",
            "vertex_id", "primitive_id", "instance_id", "is_front_face", "sample_index",
            "finalQuadUeq0EdgeTessFactor", "finalQuadVeq0EdgeTessFactor", "finalQuadUeq1EdgeTessFactor", "finalQuadVeq1EdgeTessFactor",
            "finalQuadUInsideTessFactor", "finalQuadVInsideTessFactor",
            "finalTriUeq0EdgeTessFactor", "finalTriVeq0EdgeTessFactor", "finalTriWeq0EdgeTessFactor", "finalTriInsideTessFactor",
            "finalLineDetailTessFactor", "finalLineDensityTessFactor",
        ];
        private static readonly string[] InterpolationNames =
        [
            "undefined", "constant", "linear", "linear centroid", "linear noperspective", "linear noperspective centroid",
            "linear sample", "linear noperspective sample",
        ];
        private static readonly string[] GlobalFlagNames =
        [
            "refactoringAllowed", "enableDoublePrecisionFloatOps", "forceEarlyDepthStencil", "enableRawAndStructuredBuffers",
            "skipOptimization", "enableMinimumPrecision", "enable11_1DoubleExtensions", "enable11_1ShaderExtensions",
        ];
        private const string Components = "xyzw";

        [ThreadStatic]
        private static Dictionary<uint, uint>? _structuredStrides;

        private static uint DeclaredStride(Instruction ins)
        {
            var resource = ins.Operands.FirstOrDefault(o => o.Type == OperandType.Resource);
            if (resource != null && _structuredStrides != null && _structuredStrides.TryGetValue((uint)resource.ImmediateIndex0, out var stride))
                return stride;
            return ins.ExtendedStride;
        }

        public static string Disassemble(Sm5Program program)
        {
            _structuredStrides = program.Instructions
                .Where(i => i.Opcode == Opcode.DclResourceStructured)
                .ToDictionary(i => (uint)i.Operands[0].ImmediateIndex0, i => i.Data[0]);
            var sb = new StringBuilder();
            sb.Append(StageNames[(int)program.Stage]).Append('_').Append(program.Major).Append('_').Append(program.Minor).Append('\n');
            int indent = 0;
            foreach (var ins in program.Instructions)
            {
                if (ins.Opcode is Opcode.Endif or Opcode.Endloop or Opcode.Endswitch or Opcode.Else)
                    indent = Math.Max(0, indent - 1);
                sb.Append(' ', indent * 4);
                sb.Append(Format(ins)).Append('\n');
                if (ins.Opcode is Opcode.If or Opcode.Loop or Opcode.Switch or Opcode.Else)
                    indent++;
            }
            return sb.ToString();
        }

        public static string Format(Instruction ins)
        {
            if (ins.Opcode == Opcode.CustomData) return FormatCustomData(ins);
            var sb = new StringBuilder(OpcodeInfo.Name(ins.Opcode) == "breakc" ? "breakp" : OpcodeInfo.Name(ins.Opcode));
            AppendModifiers(sb, ins);
            var args = new List<string>();
            AppendDeclarationHead(sb, ins);
            foreach (var op in ins.Operands) args.Add(FormatOperand(op, suppressComponents: ins.Opcode == Opcode.DclConstantBuffer));
            AppendDeclarationTail(args, ins);
            if (args.Count > 0)
                sb.Append(' ').Append(JoinArgs(ins, args));
            return sb.ToString();
        }

        private static string JoinArgs(Instruction ins, List<string> args) =>
            ins.Opcode == Opcode.DclIndexRange ? string.Join(' ', args) : string.Join(", ", args);

        private static void AppendModifiers(StringBuilder sb, Instruction ins)
        {
            var op = ins.Opcode;
            if (op == Opcode.Resinfo)
                sb.Append(ins.ResinfoReturn switch { ResinfoReturnType.ReciprocalFloat => "_rcpFloat", ResinfoReturnType.UnsignedInt => "_uint", _ => "" });
            if (op == Opcode.Sync)
            {
                uint f = ins.SyncFlags;
                if ((f & 8) != 0) sb.Append("_uglobal");
                if ((f & 2) != 0) sb.Append("_g");
                if ((f & 4) != 0) sb.Append("_ugroup");
                if ((f & 1) != 0) sb.Append("_t");
            }
            if (op is Opcode.If or Opcode.Breakc or Opcode.Callc or Opcode.Continuec or Opcode.Discard or Opcode.Retc)
                sb.Append(ins.TestNonZero ? "_nz" : "_z");
            if (ins.Saturate) sb.Append("_sat");
            if (ins.ExtendedDimension != ResourceDimension.Unknown)
            {
                sb.Append("_indexable(").Append(DimensionNames[(int)ins.ExtendedDimension]);
                uint stride = ins.ExtendedDimension == ResourceDimension.StructuredBuffer ? DeclaredStride(ins) : 0;
                if (stride != 0) sb.Append(", stride=").Append(stride);
                sb.Append(')');
            }
            if (ins.HasSampleOffset && ins.HasNonZeroSampleOffset)
                sb.Append('(').Append(ins.OffsetU).Append(',').Append(ins.OffsetV).Append(',').Append(ins.OffsetW).Append(')');
            if (ins.ExtendedReturnType.Count > 0 && ins.ExtendedReturnType.Any(t => t != ResourceReturnType.Real32))
                sb.Append('(').Append(string.Join(',', ins.ExtendedReturnType.Select(t => ReturnTypeNames[(int)t]))).Append(')');
        }

        private static void AppendDeclarationHead(StringBuilder sb, Instruction ins)
        {
            switch (ins.Opcode)
            {
                case Opcode.DclResource:
                    sb.Append('_').Append(DimensionNames[(int)ins.Dimension]);
                    uint samples = ins.Control >> 5 & 0x7F;
                    if (samples != 0) sb.Append('(').Append(samples).Append(')');
                    sb.Append(' ').Append(FormatReturnType(ins.Data[0]));
                    break;
                case Opcode.DclUavTyped:
                    sb.Append('_').Append(DimensionNames[(int)ins.Dimension]);
                    sb.Append(' ').Append(FormatReturnType(ins.Data[0]));
                    break;
                case Opcode.DclInputPs:
                case Opcode.DclInputPsSiv:
                    sb.Append(' ').Append(InterpolationNames[(int)ins.Interpolation]);
                    break;
                case Opcode.DclGlobalFlags:
                    sb.Append(' ').Append(FormatGlobalFlags(ins.Control));
                    break;
            }
        }

        private static void AppendDeclarationTail(List<string> args, Instruction ins)
        {
            switch (ins.Opcode)
            {
                case Opcode.DclConstantBuffer:
                    args.Add(ins.DynamicIndexed ? "dynamicIndexed" : "immediateIndexed");
                    break;
                case Opcode.DclSampler:
                    if (ins.SamplerKind == SamplerMode.Comparison) args.Add("comparisonMode");
                    else if (ins.SamplerKind == SamplerMode.Mono) args.Add("monoMode");
                    break;
                case Opcode.DclInputSgv:
                case Opcode.DclInputSiv:
                case Opcode.DclInputPsSgv:
                case Opcode.DclInputPsSiv:
                case Opcode.DclOutputSgv:
                case Opcode.DclOutputSiv:
                    args.Add(FormatSystemValue(ins.Data[0]));
                    break;
                case Opcode.DclIndexRange:
                    args.Add(ins.Data[0].ToString(CultureInfo.InvariantCulture));
                    break;
                case Opcode.DclResourceStructured:
                case Opcode.DclUavStructured:
                    args.Add(ins.Data[0].ToString(CultureInfo.InvariantCulture));
                    break;
                case Opcode.DclTgsmRaw:
                    args.Add(ins.Data[0].ToString(CultureInfo.InvariantCulture));
                    break;
                case Opcode.DclTgsmStructured:
                    args.Add(ins.Data[0].ToString(CultureInfo.InvariantCulture));
                    args.Add(ins.Data[1].ToString(CultureInfo.InvariantCulture));
                    break;
                case Opcode.DclTemps:
                case Opcode.DclHsForkPhaseInstanceCount:
                case Opcode.DclHsJoinPhaseInstanceCount:
                case Opcode.DclMaxOutputVertexCount:
                case Opcode.DclGsInstanceCount:
                    args.Add(ins.Data[0].ToString(CultureInfo.InvariantCulture));
                    break;
                case Opcode.DclHsMaxTessfactor:
                    args.Add(FormatFloat(ins.Data[0]));
                    break;
                case Opcode.DclThreadGroup:
                    args.AddRange(ins.Data.Select(d => d.ToString(CultureInfo.InvariantCulture)));
                    break;
                case Opcode.DclIndexableTemp:
                    args.Add($"x{ins.Data[0]}[{ins.Data[1]}]");
                    args.Add(ins.Data[2].ToString(CultureInfo.InvariantCulture));
                    break;
                case Opcode.DclInputControlPointCount:
                case Opcode.DclOutputControlPointCount:
                    args.Add((ins.Control & 0x3F).ToString(CultureInfo.InvariantCulture));
                    break;
                case Opcode.DclTessDomain:
                    args.Add((ins.Control & 3) switch { 1 => "domain_isoline", 2 => "domain_tri", 3 => "domain_quad", _ => "domain_undefined" });
                    break;
                case Opcode.DclTessPartitioning:
                    args.Add((ins.Control & 7) switch
                    {
                        1 => "partitioning_integer",
                        2 => "partitioning_pow2",
                        3 => "partitioning_fractional_odd",
                        4 => "partitioning_fractional_even",
                        _ => "partitioning_undefined",
                    });
                    break;
                case Opcode.DclTessOutputPrimitive:
                    args.Add((ins.Control & 7) switch
                    {
                        1 => "output_point",
                        2 => "output_line",
                        3 => "output_triangle_cw",
                        4 => "output_triangle_ccw",
                        _ => "output_undefined",
                    });
                    break;
            }
        }

        private static string FormatCustomData(Instruction ins)
        {
            if (ins.Control != CustomDataClass.ImmediateConstantBuffer)
                return $"customdata class {ins.Control} ({ins.Data.Count} dwords)";
            var sb = new StringBuilder("dcl_immediateConstantBuffer {\n");
            for (int i = 0; i + 3 < ins.Data.Count; i += 4)
            {
                sb.Append("    {").Append(string.Join(", ", Enumerable.Range(i, 4).Select(j => Hex(ins.Data[j]))))
                    .Append("},\n");
            }
            return sb.Append('}').ToString();
        }

        private static string FormatGlobalFlags(uint control)
        {
            var names = new List<string>();
            for (int i = 0; i < GlobalFlagNames.Length; i++)
                if ((control >> i & 1) != 0) names.Add(GlobalFlagNames[i]);
            return string.Join(" | ", names);
        }

        private static string FormatReturnType(uint token) =>
            "(" + string.Join(',', Enumerable.Range(0, 4).Select(i => ReturnTypeNames[Math.Min(token >> (4 * i) & 0xF, 9)])) + ")";

        private static string FormatSystemValue(uint v) => v < SystemValueNames.Length ? SystemValueNames[v] : $"sv{v}";

        private static string FormatFloat(uint bits) =>
            BitConverter.UInt32BitsToSingle(bits).ToString("0.00000000e+00", CultureInfo.InvariantCulture);

        private static string Hex(uint v) => "0x" + v.ToString("x8", CultureInfo.InvariantCulture);

        public static string FormatOperand(Operand op, bool suppressComponents = false)
        {
            var sb = new StringBuilder();
            if (op.Modifier is OperandModifier.Negate or OperandModifier.AbsoluteNegate) sb.Append('-');
            if (op.Modifier is OperandModifier.Absolute or OperandModifier.AbsoluteNegate) sb.Append('|');
            AppendRegister(sb, op);
            if (!suppressComponents) AppendComponents(sb, op);
            if (op.Modifier is OperandModifier.Absolute or OperandModifier.AbsoluteNegate) sb.Append('|');
            return sb.ToString();
        }

        private static void AppendRegister(StringBuilder sb, Operand op)
        {
            if (op.IsImmediate)
            {
                sb.Append(op.Type == OperandType.Immediate64 ? "d(" : "l(");
                sb.Append(string.Join(", ", op.Immediate.Select(Hex))).Append(')');
                return;
            }
            sb.Append(RegisterName(op.Type));
            AppendIndices(sb, op);
        }

        private static bool FirstIndexIsPlain(Operand op) => op.Indices.Count switch
        {
            0 => false,
            1 => op.Indices[0].Relative == null && op.Type is OperandType.Temp or OperandType.Output or OperandType.Sampler
                or OperandType.Resource or OperandType.UnorderedAccessView or OperandType.ThreadGroupSharedMemory
                or OperandType.IndexableTemp or OperandType.ConstantBuffer or OperandType.Input or OperandType.InputPatchConstant
                or OperandType.Label or OperandType.Stream or OperandType.FunctionBody or OperandType.FunctionTable
                or OperandType.Interface or OperandType.FunctionInput or OperandType.FunctionOutput,
            _ => op.Indices[0].Relative == null && op.Type is OperandType.IndexableTemp or OperandType.ConstantBuffer
                or OperandType.Output or OperandType.Temp,
        };

        private static void AppendIndices(StringBuilder sb, Operand op)
        {
            for (int i = 0; i < op.Indices.Count; i++)
            {
                var index = op.Indices[i];
                if (i == 0 && FirstIndexIsPlain(op))
                {
                    sb.Append(index.Immediate);
                    continue;
                }
                sb.Append('[');
                if (index.Relative != null)
                {
                    sb.Append(FormatOperand(index.Relative));
                    sb.Append(" + ").Append(index.Immediate);
                }
                else
                {
                    sb.Append(index.Immediate);
                }
                sb.Append(']');
            }
        }

        private static void AppendComponents(StringBuilder sb, Operand op)
        {
            if (op.IsImmediate || op.ComponentCount != 4) return;
            sb.Append('.');
            switch (op.Mode)
            {
                case SelectionMode.Mask:
                    for (int i = 0; i < 4; i++)
                        if ((op.Mask >> i & 1) != 0) sb.Append(Components[i]);
                    break;
                case SelectionMode.Swizzle:
                    if (op.Swizzle.All(c => c == op.Swizzle[0])) sb.Append(Components[op.Swizzle[0]]);
                    else for (int i = 0; i < 4; i++) sb.Append(Components[op.Swizzle[i]]);
                    break;
                default:
                    sb.Append(Components[op.Select1]);
                    break;
            }
        }

        private static string RegisterName(OperandType type) => type switch
        {
            OperandType.Temp => "r",
            OperandType.Input => "v",
            OperandType.Output => "o",
            OperandType.IndexableTemp => "x",
            OperandType.Sampler => "s",
            OperandType.Resource => "t",
            OperandType.ConstantBuffer => "cb",
            OperandType.ImmediateConstantBuffer => "icb",
            OperandType.Label => "l",
            OperandType.InputPrimitiveId => "vPrim",
            OperandType.OutputDepth => "oDepth",
            OperandType.Null => "null",
            OperandType.Rasterizer => "rasterizer",
            OperandType.OutputCoverageMask => "oMask",
            OperandType.Stream => "m",
            OperandType.FunctionBody => "fb",
            OperandType.FunctionTable => "ft",
            OperandType.Interface => "fp",
            OperandType.FunctionInput => "fi",
            OperandType.FunctionOutput => "fo",
            OperandType.OutputControlPointId => "vOutputControlPointID",
            OperandType.InputForkInstanceId => "vForkInstanceId",
            OperandType.InputJoinInstanceId => "vJoinInstanceId",
            OperandType.InputControlPoint => "vicp",
            OperandType.OutputControlPoint => "vocp",
            OperandType.InputPatchConstant => "vpc",
            OperandType.InputDomainPoint => "vDomainLocation",
            OperandType.ThisPointer => "this",
            OperandType.UnorderedAccessView => "u",
            OperandType.ThreadGroupSharedMemory => "g",
            OperandType.InputThreadId => "vThreadID",
            OperandType.InputThreadGroupId => "vThreadGroupID",
            OperandType.InputThreadIdInGroup => "vThreadIDInGroup",
            OperandType.InputCoverageMask => "vCoverageMask",
            OperandType.InputThreadIdInGroupFlattened => "vThreadIDInGroupFlattened",
            OperandType.InputGsInstanceId => "vGSInstanceID",
            OperandType.OutputDepthGreaterEqual => "oDepthGE",
            OperandType.OutputDepthLessEqual => "oDepthLE",
            OperandType.CycleCounter => "vCycleCounter",
            _ => $"operand{(int)type}",
        };
    }
}
