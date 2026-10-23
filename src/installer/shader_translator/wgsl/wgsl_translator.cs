using System.Text;
using AssetConverter.ShaderTranslation.Dxbc;

namespace AssetConverter.ShaderTranslation.Wgsl
{
    /// <summary>
    /// Translates a decoded vertex, pixel or compute shader into a WGSL module fragment: interface structs, resource bindings
    /// and one function <c>fn {name}(i: {name}_in) -> {name}_out</c>. All module-scope identifiers start with the shader name.
    /// </summary>
    public static class WgslTranslator
    {
        public static EmittedShader Emit(string name, Sm5Program program, ShaderBindingMap? bindings = null, WgslEmitOptions? options = null)
        {
            ArgumentException.ThrowIfNullOrEmpty(name);
            if (program.Stage is not (ShaderStage.Vertex or ShaderStage.Pixel or ShaderStage.Compute))
                throw new WgslUnsupportedException($"{name}: {program.Stage} shaders are not supported by the WGSL emitter.");
            return new WgslEmitter(name, program, bindings ?? ShaderBindingMap.Sequential(program, name), options ?? new WgslEmitOptions()).Emit();
        }
    }

    /// <summary>D3D hardware limits the emitter enforces so that corrupt blobs cannot request unbounded declarations.</summary>
    internal static class D3dLimits
    {
        public const uint MaxTempRegisters = 4096;
        public const uint MaxIndexableTempSize = 4096;
        public const uint MaxConstantBufferVectors = 4096;
    }

    internal sealed record ResourceInfo(int Register, ResourceDimension Dimension, Kind Element, bool Raw, bool Structured, uint StrideBytes, bool UsedWithCompare);

    internal sealed partial class WgslEmitter
    {
        private readonly string _name;
        private readonly Sm5Program _program;
        private readonly ShaderBindingMap _map;
        private readonly WgslEmitOptions _options;

        private readonly List<string> _lines = [];
        private int _indent = 1;
        private int _tmp;

        private readonly Dictionary<int, uint> _cbSizes = [];
        private readonly Dictionary<int, ResourceInfo> _resources = [];
        private readonly Dictionary<int, bool> _samplers = [];
        private readonly Dictionary<int, ResourceInfo> _uavs = [];
        private readonly Dictionary<int, uint> _indexableTemps = [];
        private readonly SortedDictionary<uint, (byte Mask, uint SystemValue, InterpolationMode Interpolation)> _inputs = [];
        private readonly SortedDictionary<uint, (byte Mask, uint SystemValue, InterpolationMode Interpolation)> _outputs = [];
        private readonly SortedSet<uint> _runtimeFloatBits = [];
        private uint[]? _icb;
        private int _tempCount;
        private bool _writesDepth;
        private bool _writesCoverage;
        private bool _usesDiscard;
        private bool _exitedConditionally;
        private bool _opaqueImmediates;
        private readonly Stack<SwitchState> _switches = new();
        private readonly Dictionary<int, UavAccess> _uavAccess = [];
        private readonly Dictionary<int, uint> _tgsmWords = [];
        private readonly Dictionary<int, ResourceInfo> _tgsm = [];
        private readonly HashSet<OperandType> _computeInputs = [];
        private (uint X, uint Y, uint Z)? _threadGroup;
        private readonly HashSet<(OperandType Type, int Register)> _atomicBuffers = [];

        private string InputStruct => $"{_name}_in";
        private string OutputStruct => $"{_name}_out";
        private string IcbName => $"{_name}_icb";

        public WgslEmitter(string name, Sm5Program program, ShaderBindingMap map, WgslEmitOptions options)
        {
            _name = name;
            _program = program;
            _map = map;
            _options = options;
        }

        public EmittedShader Emit()
        {
            foreach (var ins in _program.Instructions)
                WithInstructionContext(ins, () => Prepass(ins));
            foreach (var ins in _program.Instructions)
                WithInstructionContext(ins, () => EmitInstruction(ins));
            if (_switches.Count != 0)
                throw new WgslUnsupportedException($"{_name}: unterminated switch.");
            if (_indent != 1)
                throw new WgslUnsupportedException($"{_name}: unbalanced control flow (indent {_indent}).");
            if (_lines.Count == 0 || _lines[^1].Trim() != ReturnStatement)
                EmitReturn();

            var bindings = new List<EmittedBinding>();
            string declarations = BuildBindingDeclarations(bindings);
            var inputs = BuildIo(_inputs, _program.Inputs, "v");
            var outputs = BuildIo(_outputs, _program.Outputs, "o");

            var sb = new StringBuilder();
            if (_options.ProvenanceComment != null)
                sb.Append("// ").AppendLn(_options.ProvenanceComment);
            sb.Append(BuildStruct(InputStruct, inputs, extra: InputExtras()));
            sb.Append(BuildStruct(OutputStruct, outputs, extra: OutputExtras()));
            sb.Append(BuildThreadGroupConstants());
            sb.Append(declarations);
            sb.Append(BuildThreadGroupMemory());
            sb.Append(BuildIcb());
            sb.AppendLn($"fn {_name}(i: {InputStruct}) -> {OutputStruct} {{");
            sb.AppendLn($"    var o: {OutputStruct};");
            if (_usesDiscard)
                sb.AppendLn($"    var {DiscardFlag}: bool = false;");
            for (int r = 0; r < _tempCount; r++)
                sb.AppendLn($"    var r{r}: vec4<f32>;");
            foreach (var (id, size) in _indexableTemps.OrderBy(k => k.Key))
                sb.AppendLn($"    var x{id}: array<vec4<f32>, {size}>;");
            foreach (uint bits in _runtimeFloatBits)
                sb.AppendLn($"    var {RuntimeBitsName(bits)}: u32 = 0x{bits:x8}u;");
            foreach (var line in _lines) sb.AppendLn(line);
            sb.AppendLn("}");

            return new EmittedShader(_name, _program.Stage, sb.ToString(), InputStruct, OutputStruct, inputs, outputs, bindings, _threadGroup);
        }

        private List<string> InputExtras()
        {
            return [.. ComputeInputFields()];
        }

        private List<string> OutputExtras()
        {
            var extra = new List<string>();
            if (_writesDepth) extra.Add("oDepth: f32");
            if (_writesCoverage) extra.Add("oMask: u32");
            return extra;
        }

        private const string ReturnStatement = "return o;";
        private const string DiscardFlag = "discard_pending";

        private static string RuntimeBitsName(uint bits) => $"bits_{bits:x8}";

        private void WithInstructionContext(Instruction ins, Action action)
        {
            try
            {
                action();
            }
            catch (Exception ex) when (ex is not (ShaderTranslationException or OutOfMemoryException))
            {
                throw new WgslUnsupportedException(
                    $"{_name}: internal error while translating '{Sm5Disassembler.Format(ins)}' ({ex.GetType().Name}: {ex.Message}).", ex);
            }
        }

        private static uint ConstantBufferVectorCount(Instruction ins)
        {
            var size = ins.Operands[0].Indices[1].Immediate;
            if (size == 0) return D3dLimits.MaxConstantBufferVectors;
            return size <= D3dLimits.MaxConstantBufferVectors ? (uint)size : throw new WgslUnsupportedException(
                $"constant buffer declares {size} vectors; the D3D limit is {D3dLimits.MaxConstantBufferVectors}.");
        }

        private void Prepass(Instruction ins)
        {
            switch (ins.Opcode)
            {
                case Opcode.DclConstantBuffer:
                    _cbSizes[ins.Operands[0].ImmediateIndex0] = ConstantBufferVectorCount(ins);
                    break;
                case Opcode.DclSampler:
                    _samplers[ins.Operands[0].ImmediateIndex0] = ins.SamplerKind == SamplerMode.Comparison;
                    break;
                case Opcode.DclResource:
                    _resources[ins.Operands[0].ImmediateIndex0] =
                        new ResourceInfo(ins.Operands[0].ImmediateIndex0, ins.Dimension, ElementKind(ins.Data[0]), false, false, 0, false);
                    break;
                case Opcode.DclResourceRaw:
                    _resources[ins.Operands[0].ImmediateIndex0] =
                        new ResourceInfo(ins.Operands[0].ImmediateIndex0, ResourceDimension.RawBuffer, Kind.U32, true, false, 0, false);
                    break;
                case Opcode.DclResourceStructured:
                    _resources[ins.Operands[0].ImmediateIndex0] =
                        new ResourceInfo(ins.Operands[0].ImmediateIndex0, ResourceDimension.StructuredBuffer, Kind.U32, false, true, ins.Data[0], false);
                    break;
                case Opcode.DclUavRaw:
                    _uavs[ins.Operands[0].ImmediateIndex0] =
                        new ResourceInfo(ins.Operands[0].ImmediateIndex0, ResourceDimension.RawBuffer, Kind.U32, true, false, 0, false);
                    break;
                case Opcode.DclUavStructured:
                    _uavs[ins.Operands[0].ImmediateIndex0] =
                        new ResourceInfo(ins.Operands[0].ImmediateIndex0, ResourceDimension.StructuredBuffer, Kind.U32, false, true, ins.Data[0], false);
                    break;
                case Opcode.DclUavTyped:
                    _uavs[ins.Operands[0].ImmediateIndex0] =
                        new ResourceInfo(ins.Operands[0].ImmediateIndex0, ins.Dimension, ElementKind(ins.Data[0]), false, false, 0, false);
                    break;
                case Opcode.DclTgsmRaw:
                    DeclareThreadGroupMemory(ins, raw: true);
                    break;
                case Opcode.DclTgsmStructured:
                    DeclareThreadGroupMemory(ins, raw: false);
                    break;
                case Opcode.DclThreadGroup:
                    DeclareThreadGroup(ins);
                    break;
                case Opcode.AtomicAnd:
                case Opcode.AtomicOr:
                case Opcode.AtomicXor:
                case Opcode.AtomicIadd:
                case Opcode.AtomicUmax:
                case Opcode.AtomicUmin:
                case Opcode.AtomicImax:
                case Opcode.AtomicImin:
                case Opcode.AtomicCmpStore:
                    _atomicBuffers.Add(AtomicKey(ins.Operands[0]));
                    break;
                case Opcode.ImmAtomicIadd:
                case Opcode.ImmAtomicAnd:
                case Opcode.ImmAtomicOr:
                case Opcode.ImmAtomicXor:
                case Opcode.ImmAtomicExch:
                case Opcode.ImmAtomicCmpExch:
                case Opcode.ImmAtomicImax:
                case Opcode.ImmAtomicImin:
                case Opcode.ImmAtomicUmax:
                case Opcode.ImmAtomicUmin:
                    _atomicBuffers.Add(AtomicKey(ins.Operands[1]));
                    break;
                case Opcode.StoreUavTyped:
                    NoteUavAccess(ins.Operands[0], UavAccess.Write);
                    break;
                case Opcode.LdUavTyped:
                    NoteUavAccess(ins.Operands[2], UavAccess.Read);
                    break;
                case Opcode.DclTemps:
                    if (ins.Data[0] > D3dLimits.MaxTempRegisters)
                        throw new WgslUnsupportedException($"{_name}: dcl_temps {ins.Data[0]} exceeds the D3D limit of {D3dLimits.MaxTempRegisters}.");
                    _tempCount = (int)ins.Data[0];
                    break;
                case Opcode.DclIndexableTemp:
                    if (ins.Data[0] >= D3dLimits.MaxTempRegisters || ins.Data[1] is 0 or > D3dLimits.MaxIndexableTempSize)
                        throw new WgslUnsupportedException($"{_name}: dcl_indexableTemp x{ins.Data[0]}[{ins.Data[1]}] is outside the D3D limits.");
                    _indexableTemps[(int)ins.Data[0]] = ins.Data[1];
                    break;
                case Opcode.DclInput:
                case Opcode.DclInputPs:
                case Opcode.DclInputSgv:
                case Opcode.DclInputSiv:
                case Opcode.DclInputPsSgv:
                case Opcode.DclInputPsSiv:
                    RecordIo(_inputs, ins);
                    break;
                case Opcode.DclOutput:
                case Opcode.DclOutputSgv:
                case Opcode.DclOutputSiv:
                    RecordIo(_outputs, ins);
                    break;
                case Opcode.CustomData when ins.Control == CustomDataClass.ImmediateConstantBuffer:
                    _icb = ins.Data.ToArray();
                    break;
                case Opcode.Discard:
                    _usesDiscard = true;
                    break;
                case Opcode.DclGlobalFlags:
                case Opcode.DclIndexRange:
                    break;
                case Opcode.HsDecls:
                case Opcode.DclTessDomain:
                    throw new WgslUnsupportedException($"{_name}: {OpcodeInfo.Name(ins.Opcode)} is not valid in a vertex or pixel shader.");
                case Opcode.SampleC:
                case Opcode.SampleCLz:
                case Opcode.Gather4C:
                    MarkCompare(ins);
                    break;
                default:
                    if (OpcodeInfo.IsDeclaration(ins.Opcode))
                        throw new WgslUnsupportedException($"{_name}: declaration {OpcodeInfo.Name(ins.Opcode)} is not supported.");
                    break;
            }
        }

        private void MarkCompare(Instruction ins)
        {
            var res = SourceOfType(ins, OperandType.Resource);
            int reg = res.ImmediateIndex0;
            if (_resources.TryGetValue(reg, out var info))
                _resources[reg] = info with { UsedWithCompare = true };
        }

        private static Kind ElementKind(uint returnTypeToken) => (ResourceReturnType)(returnTypeToken & 0xF) switch
        {
            ResourceReturnType.SignedInt => Kind.I32,
            ResourceReturnType.UnsignedInt => Kind.U32,
            _ => Kind.F32,
        };

        private void RecordIo(SortedDictionary<uint, (byte Mask, uint SystemValue, InterpolationMode Interpolation)> target, Instruction ins)
        {
            var op = ins.Operands[0];
            if (IsComputeInput(op.Type))
            {
                RecordComputeInput(op.Type);
                return;
            }
            if (op.Type is OperandType.OutputDepth or OperandType.OutputDepthGreaterEqual or OperandType.OutputDepthLessEqual)
            {
                _writesDepth = true;
                return;
            }
            if (op.Type == OperandType.OutputCoverageMask)
            {
                _writesCoverage = true;
                return;
            }
            if (op.Type is not (OperandType.Input or OperandType.Output) || op.Indices.Count != 1)
                throw new WgslUnsupportedException($"{_name}: unsupported I/O declaration {Sm5Disassembler.Format(ins)}.");
            uint reg = (uint)op.Indices[0].Immediate;
            byte mask = op.Mode == SelectionMode.Mask ? op.Mask : (byte)15;
            uint sys = ins.Data.Count > 0 ? ins.Data[0] : 0;
            target.TryGetValue(reg, out var existing);
            var interp = ins.Opcode is Opcode.DclInputPs or Opcode.DclInputPsSiv or Opcode.DclInputPsSgv ? ins.Interpolation : existing.Interpolation;
            target[reg] = ((byte)(existing.Mask | mask), sys != 0 ? sys : existing.SystemValue, interp);
        }

        private static List<IoRegister> BuildIo(
            SortedDictionary<uint, (byte Mask, uint SystemValue, InterpolationMode Interpolation)> declared,
            IReadOnlyList<SignatureElement> signature, string prefix)
        {
            var result = new List<IoRegister>();
            foreach (var (reg, info) in declared)
            {
                var sig = signature.FirstOrDefault(s => s.Register == reg);
                string semantic = sig == null ? "" : $"{sig.SemanticName}{sig.SemanticIndex}";
                var elements = signature
                    .Where(s => s.Register == reg && (s.Mask & info.Mask) != 0)
                    .Select(s => new IoElement(s.SemanticName, s.SemanticIndex, (byte)(s.Mask & info.Mask)))
                    .ToList();
                result.Add(new IoRegister($"{prefix}{reg}", reg, info.Mask, semantic, info.SystemValue, info.Interpolation, elements));
            }
            return result;
        }

        private static string BuildStruct(string name, List<IoRegister> fields, List<string> extra)
        {
            var sb = new StringBuilder();
            sb.AppendLn($"struct {name} {{");
            foreach (var f in fields)
            {
                string comment = f.Semantic.Length > 0 ? $" // {f.Semantic} mask=0x{f.Mask:x}" : $" // mask=0x{f.Mask:x}";
                sb.AppendLn($"    {f.Field}: vec4<f32>,{comment}");
            }
            foreach (var e in extra) sb.AppendLn($"    {e},");
            if (fields.Count == 0 && extra.Count == 0) sb.AppendLn("    unused: u32,");
            sb.AppendLn("}");
            return sb.ToString();
        }

        private string BuildIcb()
        {
            if (_icb == null) return "";
            int count = _icb.Length / 4;
            var rows = Enumerable.Range(0, count)
                .Select(r => $"vec4<u32>({string.Join(", ", Enumerable.Range(0, 4).Select(c => $"0x{_icb[r * 4 + c]:x8}u"))})");
            return $"var<private> {IcbName}: array<vec4<u32>, {count}> = array<vec4<u32>, {count}>({string.Join(", ", rows)});\n";
        }

        private ResourceBinding Binding(IDictionary<int, ResourceBinding> table, string kind, int register)
        {
            if (!table.TryGetValue(register, out var binding))
                throw new UnmappedResourceException($"{_name}: no binding mapped for {kind}{register}.");
            return binding;
        }

        private string BuildBindingDeclarations(List<EmittedBinding> bindings)
        {
            var sb = new StringBuilder();
            void Declare(string kind, int register, ResourceBinding b, string type, string storage)
            {
                bindings.Add(new EmittedBinding(kind, register, b.Name, b.Group, b.Binding, type, !b.External));
                if (!b.External)
                    sb.AppendLn($"@group({b.Group}) @binding({b.Binding}) var{storage} {b.Name}: {type};");
            }

            foreach (var (reg, size) in _cbSizes.OrderBy(k => k.Key))
                Declare("cb", reg, Binding(_map.ConstantBuffers, "cb", reg), $"array<vec4<f32>, {size}>", "<uniform>");
            foreach (var (reg, comparison) in _samplers.OrderBy(k => k.Key))
                Declare("s", reg, Binding(_map.Samplers, "s", reg), comparison ? "sampler_comparison" : "sampler", "");
            foreach (var (reg, info) in _resources.OrderBy(k => k.Key))
            {
                var b = Binding(_map.Resources, "t", reg);
                if (info.Raw || info.Structured)
                    Declare("t", reg, b, "array<u32>", "<storage, read>");
                else
                    Declare("t", reg, b, TextureType(info, b), "");
            }
            foreach (var (reg, info) in _uavs.OrderBy(k => k.Key))
            {
                var b = Binding(_map.UnorderedAccess, "u", reg);
                if (info.Raw || info.Structured)
                    Declare("u", reg, b, IsAtomicBuffer(OperandType.UnorderedAccessView, reg) ? "array<atomic<u32>>" : "array<u32>", "<storage, read_write>");
                else
                    Declare("u", reg, b, StorageTextureType(info, b, reg), "");
            }
            return sb.ToString();
        }

        private static bool IsDepth(ResourceInfo info, ResourceBinding binding) =>
            binding.TextureOverride == TextureOverride.Depth || (binding.TextureOverride == TextureOverride.None && info.UsedWithCompare);

        private string TextureType(ResourceInfo info, ResourceBinding binding)
        {
            bool depth = IsDepth(info, binding);
            string element = WgslText.Scalar(info.Element);
            return info.Dimension switch
            {
                ResourceDimension.Texture1D => depth ? throw Unsupported("1D depth texture") : $"texture_1d<{element}>",
                ResourceDimension.Texture2D => depth ? "texture_depth_2d" : $"texture_2d<{element}>",
                ResourceDimension.Texture2DMs => depth ? "texture_depth_multisampled_2d" : $"texture_multisampled_2d<{element}>",
                ResourceDimension.Texture3D => depth ? throw Unsupported("3D depth texture") : $"texture_3d<{element}>",
                ResourceDimension.TextureCube => depth ? "texture_depth_cube" : $"texture_cube<{element}>",
                ResourceDimension.Texture2DArray => depth ? "texture_depth_2d_array" : $"texture_2d_array<{element}>",
                ResourceDimension.TextureCubeArray => depth ? "texture_depth_cube_array" : $"texture_cube_array<{element}>",
                _ => throw Unsupported($"resource dimension {info.Dimension}"),
            };
        }

        private WgslUnsupportedException Unsupported(string what) => new($"{_name}: {what} is not supported.");

        private Operand SourceOfType(Instruction ins, OperandType type) =>
            ins.Operands.FirstOrDefault(o => o.Type == type)
                ?? throw new WgslUnsupportedException($"{_name}: '{Sm5Disassembler.Format(ins)}' has no {type} operand.");

        private void EmitReturn()
        {
            if (_usesDiscard)
                Line($"if ({DiscardFlag}) {{ discard; }}");
            Line(ReturnStatement);
        }

        private void Line(string text)
        {
            if (_switches.Count > 0) _switches.Peek().NoteStatement(text);
            _lines.Add(new string(' ', _indent * 4) + text);
        }

        private string NewTemp() => $"tmp{_tmp++}";

        private Val Bind(Val value)
        {
            string name = NewTemp();
            Line($"let {name} = {value.Text};");
            return value with { Text = name };
        }

        private sealed class SwitchState
        {
            public int HeaderLine = -1;
            public List<string> Selectors = [];
            public bool ClauseOpen;
            public bool ClauseEmpty;
            public bool LastTerminated;
            public bool SawDefault;
            public int ClauseIndent;

            public void NoteStatement(string text)
            {
                ClauseEmpty = false;
                LastTerminated = text is "break;" or ReturnStatement or "continue;";
            }
        }
    }
}
