using System.Text;
using AssetConverter.ShaderTranslation.Dxbc;

namespace AssetConverter.ShaderTranslation.Wgsl
{
    [Flags]
    internal enum UavAccess { None = 0, Read = 1, Write = 2 }

    /// <summary>WGSL storage texel formats usable for typed UAVs, with the scalar kind their texels read and write as.</summary>
    public static class WgslStorageFormats
    {
        private static readonly Dictionary<string, Kind> Formats = new(StringComparer.Ordinal)
        {
            ["rgba8unorm"] = Kind.F32,
            ["rgba8snorm"] = Kind.F32,
            ["rgba16float"] = Kind.F32,
            ["r32float"] = Kind.F32,
            ["rg32float"] = Kind.F32,
            ["rgba32float"] = Kind.F32,
            ["bgra8unorm"] = Kind.F32,
            ["rgba8uint"] = Kind.U32,
            ["rgba16uint"] = Kind.U32,
            ["r32uint"] = Kind.U32,
            ["rg32uint"] = Kind.U32,
            ["rgba32uint"] = Kind.U32,
            ["rgba8sint"] = Kind.I32,
            ["rgba16sint"] = Kind.I32,
            ["r32sint"] = Kind.I32,
            ["rg32sint"] = Kind.I32,
            ["rgba32sint"] = Kind.I32,
        };

        public static bool IsKnown(string format) => Formats.ContainsKey(format);

        internal static Kind ScalarKind(string format) => Formats[format];
    }

    internal sealed partial class WgslEmitter
    {
        private const uint SyncThreadsInGroup = 1;
        private const uint SyncSharedMemory = 2;
        private const uint SyncUavGroup = 4;
        private const uint SyncUavGlobal = 8;

        private static bool IsComputeInput(OperandType type) =>
            type is OperandType.InputThreadId or OperandType.InputThreadGroupId
                or OperandType.InputThreadIdInGroup or OperandType.InputThreadIdInGroupFlattened;

        private static string ComputeInputField(OperandType type) => type switch
        {
            OperandType.InputThreadId => "vThreadId",
            OperandType.InputThreadGroupId => "vGroupId",
            OperandType.InputThreadIdInGroup => "vThreadIdInGroup",
            _ => "vThreadIndex",
        };

        private void RecordComputeInput(OperandType type)
        {
            if (_program.Stage != ShaderStage.Compute)
                throw new WgslUnsupportedException($"{_name}: {type} is only valid in compute shaders.");
            _computeInputs.Add(type);
        }

        private IEnumerable<string> ComputeInputFields()
        {
            foreach (var type in new[]
            {
                OperandType.InputThreadId, OperandType.InputThreadGroupId, OperandType.InputThreadIdInGroup,
                OperandType.InputThreadIdInGroupFlattened,
            })
            {
                if (_computeInputs.Contains(type))
                    yield return $"{ComputeInputField(type)}: {(type == OperandType.InputThreadIdInGroupFlattened ? "u32" : "vec3<u32>")}";
            }
        }

        private Val ReadComputeInput(Operand op, List<int> slots, Kind want)
        {
            if (!_computeInputs.Contains(op.Type))
                throw new WgslUnsupportedException($"{_name}: {Sm5Disassembler.FormatOperand(op)} is read but not declared.");
            string field = $"i.{ComputeInputField(op.Type)}";
            Val value;
            if (op.Type == OperandType.InputThreadIdInGroupFlattened)
            {
                value = new Val(slots.Count == 1 ? field : $"vec{slots.Count}<u32>({field})", Kind.U32, slots.Count);
            }
            else
            {
                string widened = $"vec4<u32>({field}, 0u)";
                value = new Val(WgslText.Swizzle(widened, slots.Select(op.SourceComponent).ToList()), Kind.U32, slots.Count);
            }
            value = ApplyModifier(value, op.Modifier);
            return want == Kind.U32 ? value : Convert(value, want);
        }

        private void DeclareThreadGroup(Instruction ins)
        {
            if (_program.Stage != ShaderStage.Compute)
                throw new WgslUnsupportedException($"{_name}: dcl_thread_group is only valid in compute shaders.");
            _threadGroup = (ins.Data[0], ins.Data[1], ins.Data[2]);
        }

        private void DeclareThreadGroupMemory(Instruction ins, bool raw)
        {
            int reg = ins.Operands[0].ImmediateIndex0;
            uint stride = raw ? 0 : ins.Data[0];
            uint bytes = raw ? ins.Data[0] : ins.Data[0] * ins.Data[1];
            if (bytes % 4 != 0 || (!raw && stride % 4 != 0))
                throw new WgslUnsupportedException($"{_name}: g{reg} is not 4-byte aligned.");
            _tgsmWords[reg] = bytes / 4;
            _tgsm[reg] = new ResourceInfo(reg, raw ? ResourceDimension.RawBuffer : ResourceDimension.StructuredBuffer, Kind.U32, raw, !raw, stride, false);
        }

        private string ThreadGroupMemoryName(int register) => $"{_name}_g{register}";

        private string BuildThreadGroupConstants()
        {
            if (_threadGroup is not var (x, y, z)) return "";
            var sb = new StringBuilder();
            sb.AppendLn($"const {_name}_workgroup_size_x: u32 = {x}u;");
            sb.AppendLn($"const {_name}_workgroup_size_y: u32 = {y}u;");
            sb.AppendLn($"const {_name}_workgroup_size_z: u32 = {z}u;");
            return sb.ToString();
        }

        private string BuildThreadGroupMemory()
        {
            var sb = new StringBuilder();
            foreach (var (reg, words) in _tgsmWords.OrderBy(k => k.Key))
                sb.AppendLn($"var<workgroup> {ThreadGroupMemoryName(reg)}: array<{(IsAtomicBuffer(OperandType.ThreadGroupSharedMemory, reg) ? "atomic<u32>" : "u32")}, {words}>;");
            return sb.ToString();
        }

        private void NoteUavAccess(Operand uav, UavAccess access)
        {
            int reg = uav.ImmediateIndex0;
            _uavAccess[reg] = _uavAccess.GetValueOrDefault(reg) | access;
        }

        private string StorageFormatOf(int register, ResourceBinding binding)
        {
            if (string.IsNullOrEmpty(binding.UavFormat))
                throw new UnmappedResourceException($"{_name}: typed UAV u{register} needs a uav_format in its binding.");
            if (!WgslStorageFormats.IsKnown(binding.UavFormat))
                throw new WgslUnsupportedException($"{_name}: u{register} storage format '{binding.UavFormat}' is not a supported WGSL storage format.");
            return binding.UavFormat;
        }

        private string StorageTextureType(ResourceInfo info, ResourceBinding binding, int register)
        {
            string format = StorageFormatOf(register, binding);
            string dimension = info.Dimension switch
            {
                ResourceDimension.Texture1D => "1d",
                ResourceDimension.Texture2D => "2d",
                ResourceDimension.Texture3D => "3d",
                ResourceDimension.Texture2DArray => "2d_array",
                _ => throw new WgslUnsupportedException($"{_name}: typed UAV u{register} of dimension {info.Dimension} is not supported."),
            };
            var access = _uavAccess.GetValueOrDefault(register) switch
            {
                UavAccess.Read => "read",
                UavAccess.Read | UavAccess.Write => "read_write",
                _ => "write",
            };
            return $"texture_storage_{dimension}<{format}, {access}>";
        }

        private (string Name, ResourceInfo Info, string Format) TypedUavOf(Operand uav)
        {
            int reg = uav.ImmediateIndex0;
            if (uav.Type != OperandType.UnorderedAccessView || !_uavs.TryGetValue(reg, out var info) || info.Raw || info.Structured)
                throw new WgslUnsupportedException($"{_name}: {Sm5Disassembler.FormatOperand(uav)} is not a typed UAV.");
            var binding = Binding(_map.UnorderedAccess, "u", reg);
            return (binding.Name, info, StorageFormatOf(reg, binding));
        }

        private string StorageCoordinates(Operand addr, ResourceInfo info, out string? layer)
        {
            int count = CoordinateCount(info.Dimension);
            layer = IsArray(info.Dimension) ? Read(addr, 1 << count, Kind.U32).Text : null;
            return Read(addr, SlotMaskOf(count), Kind.U32).Text;
        }

        private void EmitTypedUavLoad(Instruction ins)
        {
            var dst = ins.Operands[0];
            var uav = ins.Operands[2];
            var (name, info, format) = TypedUavOf(uav);
            string coords = StorageCoordinates(ins.Operands[1], info, out var layer);
            string call = layer == null ? $"textureLoad({name}, {coords})" : $"textureLoad({name}, {coords}, {layer})";
            StoreTextureResult(dst, uav, call, WgslStorageFormats.ScalarKind(format));
        }

        private void EmitTypedUavStore(Instruction ins)
        {
            var uav = ins.Operands[0];
            var (name, info, format) = TypedUavOf(uav);
            string coords = StorageCoordinates(ins.Operands[1], info, out var layer);
            string value = Read(ins.Operands[2], 0xF, WgslStorageFormats.ScalarKind(format)).Text;
            Line(layer == null
                ? $"textureStore({name}, {coords}, {value});"
                : $"textureStore({name}, {coords}, {layer}, {value});");
        }

        private static (OperandType, int) AtomicKey(Operand res) => (res.Type, res.ImmediateIndex0);

        private bool IsAtomicBuffer(OperandType type, int register) => _atomicBuffers.Contains((type, register));

        private bool IsAtomicBuffer(Operand res) => IsAtomicBuffer(res.Type, res.ImmediateIndex0);

        private static string? AtomicFunction(Opcode op) => op switch
        {
            Opcode.AtomicAnd or Opcode.ImmAtomicAnd => "atomicAnd",
            Opcode.AtomicOr or Opcode.ImmAtomicOr => "atomicOr",
            Opcode.AtomicXor or Opcode.ImmAtomicXor => "atomicXor",
            Opcode.AtomicIadd or Opcode.ImmAtomicIadd => "atomicAdd",
            Opcode.AtomicUmax or Opcode.ImmAtomicUmax => "atomicMax",
            Opcode.AtomicUmin or Opcode.ImmAtomicUmin => "atomicMin",
            Opcode.ImmAtomicExch => "atomicExchange",
            _ => null,
        };

        private bool EmitAtomic(Instruction ins)
        {
            bool returnsOld = ins.Opcode >= Opcode.ImmAtomicIadd && ins.Opcode <= Opcode.ImmAtomicUmin;
            bool plain = ins.Opcode >= Opcode.AtomicAnd && ins.Opcode <= Opcode.AtomicUmin;
            if (!returnsOld && !plain) return false;
            string? function = AtomicFunction(ins.Opcode);
            if (function == null)
                throw new WgslUnsupportedException($"{_name}: atomic '{OpcodeInfo.Name(ins.Opcode)}' is not supported (signed min/max need i32 atomics; compare-exchange is weak in WGSL).");
            int resIndex = returnsOld ? 1 : 0;
            var res = ins.Operands[resIndex];
            var (name, info) = BufferOf(res);
            var addr = ins.Operands[resIndex + 1];
            string word = info.Structured
                ? StructuredWordIndex(Read(addr, 1, Kind.U32).Text, Read(addr, 2, Kind.U32).Text, info)
                : $"({Read(addr, 1, Kind.U32).Text} >> 2u)";
            string call = $"{function}(&{name}[{word}], {Read(ins.Operands[resIndex + 2], 1, Kind.U32).Text})";
            var dst = ins.Operands[0];
            if (!returnsOld || dst.Type == OperandType.Null)
            {
                Line($"_ = {call};");
                return true;
            }
            if (Slots(DestinationMask(dst)).Count != 1)
                throw new WgslUnsupportedException($"{_name}: atomic result must be a single component.");
            Write(dst, new Val(call, Kind.U32, 1));
            return true;
        }

        /// <summary>
        /// WGSL barriers are control barriers and must be reached by the whole workgroup. A D3D sync with the thread-group flag already has
        /// that requirement; a fence-only sync is legal in divergent code, so it is only translated where uniform reachability is certain.
        /// </summary>
        private bool ReachedByWholeWorkgroup => _indent == 1 && !_exitedConditionally;

        private void EmitSync(Instruction ins)
        {
            uint flags = ins.SyncFlags;
            if ((flags & SyncUavGlobal) != 0)
                throw new WgslUnsupportedException($"{_name}: sync with a global UAV fence cannot be expressed in WGSL.");
            if (flags == 0) return;
            if ((flags & SyncThreadsInGroup) == 0 && !ReachedByWholeWorkgroup)
                throw new WgslUnsupportedException($"{_name}: a fence-only sync inside control flow cannot be expressed in WGSL (its barriers require uniform control flow).");
            Line("workgroupBarrier();");
            if ((flags & SyncUavGroup) == 0) return;
            if (_uavs.Values.Any(u => u.Raw || u.Structured))
                Line("storageBarrier();");
            if (_uavs.Values.Any(u => !u.Raw && !u.Structured))
                Line("textureBarrier();");
        }
    }
}
