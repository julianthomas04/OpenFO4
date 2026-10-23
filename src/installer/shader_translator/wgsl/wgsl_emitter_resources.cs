using AssetConverter.ShaderTranslation.Dxbc;

namespace AssetConverter.ShaderTranslation.Wgsl
{
    internal sealed partial class WgslEmitter
    {
        private bool EmitResourceInstruction(Instruction ins)
        {
            switch (ins.Opcode)
            {
                case Opcode.Sample:
                case Opcode.SampleB:
                case Opcode.SampleL:
                case Opcode.SampleD:
                case Opcode.SampleC:
                case Opcode.SampleCLz:
                case Opcode.Gather4:
                case Opcode.Gather4C:
                    EmitSample(ins);
                    return true;
                case Opcode.Ld:
                case Opcode.LdMs:
                    EmitTextureLoad(ins);
                    return true;
                case Opcode.Resinfo:
                    EmitResinfo(ins);
                    return true;
                case Opcode.LdRaw:
                case Opcode.LdStructured:
                    EmitBufferLoad(ins);
                    return true;
                case Opcode.StoreRaw:
                case Opcode.StoreStructured:
                    EmitBufferStore(ins);
                    return true;
                case Opcode.Bufinfo:
                    EmitBufferInfo(ins);
                    return true;
                case Opcode.LdUavTyped:
                    EmitTypedUavLoad(ins);
                    return true;
                case Opcode.StoreUavTyped:
                    EmitTypedUavStore(ins);
                    return true;
                default:
                    return false;
            }
        }

        private (ResourceInfo Info, ResourceBinding Binding) TextureOf(Operand res)
        {
            int reg = res.ImmediateIndex0;
            if (res.Type != OperandType.Resource)
                throw new WgslUnsupportedException($"{_name}: {Sm5Disassembler.FormatOperand(res)} is not a texture resource.");
            if (!_resources.TryGetValue(reg, out var info))
                throw new WgslUnsupportedException($"{_name}: resource t{reg} is used but not declared.");
            if (info.Raw || info.Structured)
                throw new WgslUnsupportedException($"{_name}: t{reg} is a buffer, not a texture.");
            return (info, Binding(_map.Resources, "t", reg));
        }

        /// <summary>Rejects sampling forms that WGSL has no builtin for (naga would only report them when the engine starts).</summary>
        private void RequireSamplingSupport(Opcode opcode, ResourceDimension dimension, bool depth)
        {
            string form = OpcodeInfo.Name(opcode);
            bool supported = opcode switch
            {
                Opcode.SampleB => dimension != ResourceDimension.Texture1D && !depth,
                Opcode.SampleD => dimension != ResourceDimension.Texture1D && !depth,
                Opcode.Gather4 or Opcode.Gather4C => dimension is ResourceDimension.Texture2D or ResourceDimension.Texture2DArray
                    or ResourceDimension.TextureCube or ResourceDimension.TextureCubeArray,
                Opcode.SampleC or Opcode.SampleCLz => dimension != ResourceDimension.Texture3D && dimension != ResourceDimension.Texture1D,
                _ => true,
            };
            if (!supported)
                throw Unsupported($"{form} on {dimension}{(depth ? " (depth)" : "")}");
        }

        private static int CoordinateCount(ResourceDimension dim) => dim switch
        {
            ResourceDimension.Texture1D => 1,
            ResourceDimension.Texture2D or ResourceDimension.Texture2DArray or ResourceDimension.Texture2DMs => 2,
            ResourceDimension.Texture3D or ResourceDimension.TextureCube or ResourceDimension.TextureCubeArray => 3,
            _ => throw new WgslUnsupportedException($"Resource dimension {dim} has no WGSL texture equivalent."),
        };

        private static bool IsArray(ResourceDimension dim) => dim is ResourceDimension.Texture2DArray or ResourceDimension.TextureCubeArray;

        private string? OffsetArgument(Instruction ins, ResourceDimension dim)
        {
            if (!ins.HasSampleOffset || !ins.HasNonZeroSampleOffset) return null;
            return dim switch
            {
                ResourceDimension.Texture1D => $"{ins.OffsetU}",
                ResourceDimension.Texture2D or ResourceDimension.Texture2DArray => $"vec2<i32>({ins.OffsetU}, {ins.OffsetV})",
                ResourceDimension.Texture3D => $"vec3<i32>({ins.OffsetU}, {ins.OffsetV}, {ins.OffsetW})",
                _ => throw new WgslUnsupportedException($"{_name}: texel offsets are not allowed on {dim}."),
            };
        }

        private void EmitSample(Instruction ins)
        {
            var dst = ins.Operands[0];
            var addr = ins.Operands[1];
            var res = SourceOfType(ins, OperandType.Resource);
            var smp = SourceOfType(ins, OperandType.Sampler);
            var (info, tex) = TextureOf(res);
            bool depth = IsDepth(info, tex);
            RequireSamplingSupport(ins.Opcode, info.Dimension, depth);
            int coordCount = CoordinateCount(info.Dimension);
            var coords = Read(addr, SlotMaskOf(coordCount), Kind.F32);
            int extraStart = ins.Operands.ToList().IndexOf(smp) + 1;
            var args = new List<string> { tex.Name };
            string smpName = Binding(_map.Samplers, "s", smp.ImmediateIndex0).Name;
            string? layer = IsArray(info.Dimension) ? $"i32(floor({Read(addr, 1 << coordCount, Kind.F32).Text} + 0.5))" : null;
            string? offset = OffsetArgument(ins, info.Dimension);
            Kind resultKind = Kind.F32;
            string fn;
            var middle = new List<string>();

            switch (ins.Opcode)
            {
                case Opcode.Sample:
                    fn = "textureSample";
                    break;
                case Opcode.SampleB:
                    fn = "textureSampleBias";
                    middle.Add(Read(ins.Operands[extraStart], 1, Kind.F32).Text);
                    break;
                case Opcode.SampleL:
                    fn = "textureSampleLevel";
                    string level = Read(ins.Operands[extraStart], 1, Kind.F32).Text;
                    middle.Add(depth ? $"i32({level})" : level);
                    break;
                case Opcode.SampleD:
                    fn = "textureSampleGrad";
                    middle.Add(Read(ins.Operands[extraStart], SlotMaskOf(coordCount), Kind.F32).Text);
                    middle.Add(Read(ins.Operands[extraStart + 1], SlotMaskOf(coordCount), Kind.F32).Text);
                    break;
                case Opcode.SampleC:
                    fn = "textureSampleCompare";
                    middle.Add(Read(ins.Operands[extraStart], 1, Kind.F32).Text);
                    break;
                case Opcode.SampleCLz:
                    fn = "textureSampleCompareLevel";
                    middle.Add(Read(ins.Operands[extraStart], 1, Kind.F32).Text);
                    break;
                case Opcode.Gather4:
                    fn = "textureGather";
                    resultKind = info.Element;
                    break;
                default:
                    fn = "textureGatherCompare";
                    middle.Add(Read(ins.Operands[extraStart], 1, Kind.F32).Text);
                    break;
            }

            if (ins.Opcode == Opcode.Gather4 && !depth)
                args.Insert(0, smp.SourceComponent(0).ToString(System.Globalization.CultureInfo.InvariantCulture));
            args.Add(smpName);
            args.Add(coords.Text);
            if (layer != null) args.Add(layer);
            args.AddRange(middle);
            if (offset != null) args.Add(offset);

            string call = $"{fn}({string.Join(", ", args)})";
            bool compare = ins.Opcode is Opcode.SampleC or Opcode.SampleCLz;
            if (compare)
                call = $"vec4<f32>({call})";
            else if (depth && ins.Opcode is not (Opcode.Gather4 or Opcode.Gather4C))
                call = $"vec4<f32>({call}, 0.0, 0.0, 1.0)";

            StoreTextureResult(dst, res, call, resultKind);
        }

        private void StoreTextureResult(Operand dst, Operand res, string call, Kind kind)
        {
            int mask = DestinationMask(dst);
            var slots = Slots(mask);
            var fetched = Bind(new Val(call, kind, 4));
            var picked = new Val(WgslText.Swizzle(fetched.Text, slots.Select(res.SourceComponent).ToList()), kind, slots.Count);
            Write(dst, picked);
        }

        /// <summary>The mip level of <c>ld</c> is always the fourth address component, whatever the dimension.</summary>
        private const int LoadMipLevelComponent = 3;

        private void EmitTextureLoad(Instruction ins)
        {
            var dst = ins.Operands[0];
            var addr = ins.Operands[1];
            var res = ins.Operands[2];
            var (info, tex) = TextureOf(res);
            if (info.Dimension is ResourceDimension.Buffer)
                throw new WgslUnsupportedException($"{_name}: typed buffer loads are not supported.");
            int coordCount = CoordinateCount(info.Dimension);
            bool array = IsArray(info.Dimension);
            var coords = Read(addr, SlotMaskOf(coordCount), Kind.I32).Text;
            string? offset = OffsetArgument(ins, info.Dimension);
            if (offset != null) coords = $"({coords} + {offset})";
            var args = new List<string> { tex.Name, coords };
            if (array) args.Add(Read(addr, 1 << coordCount, Kind.I32).Text);
            if (ins.Opcode == Opcode.LdMs)
                args.Add(Read(ins.Operands[3], 1, Kind.I32).Text);
            else if (info.Dimension != ResourceDimension.Texture2DMs)
                args.Add(Read(addr, 1 << LoadMipLevelComponent, Kind.I32).Text);
            string call = $"textureLoad({string.Join(", ", args)})";
            if (IsDepth(info, tex))
                call = $"vec4<f32>({call}, 0.0, 0.0, 1.0)";
            StoreTextureResult(dst, res, call, IsDepth(info, tex) ? Kind.F32 : info.Element);
        }

        private void EmitResinfo(Instruction ins)
        {
            var dst = ins.Operands[0];
            var res = ins.Operands[2];
            var (info, tex) = TextureOf(res);
            string level = Read(ins.Operands[1], 1, Kind.U32).Text;
            string t = tex.Name;
            bool ms = info.Dimension == ResourceDimension.Texture2DMs;
            string dims = ms ? $"textureDimensions({t})" : $"textureDimensions({t}, {level})";
            string levels = ms ? "0u" : $"textureNumLevels({t})";
            string components = info.Dimension switch
            {
                ResourceDimension.Texture1D => $"vec4<u32>({dims}, 0u, 0u, {levels})",
                ResourceDimension.Texture2D or ResourceDimension.TextureCube or ResourceDimension.Texture2DMs =>
                    $"vec4<u32>({dims}, 0u, {levels})",
                ResourceDimension.Texture3D => $"vec4<u32>({dims}, {levels})",
                ResourceDimension.Texture2DArray or ResourceDimension.TextureCubeArray =>
                    $"vec4<u32>({dims}, textureNumLayers({t}), {levels})",
                _ => throw new WgslUnsupportedException($"{_name}: resinfo on {info.Dimension} is not supported."),
            };
            var bound = Bind(new Val(components, Kind.U32, 4));
            Val converted = ins.ResinfoReturn switch
            {
                ResinfoReturnType.UnsignedInt => bound,
                ResinfoReturnType.ReciprocalFloat => new Val(ReciprocalDimensions(bound.Text, ResinfoDimensionLanes(info.Dimension)), Kind.F32, 4),
                _ => new Val($"vec4<f32>({bound.Text})", Kind.F32, 4),
            };
            var slots = Slots(DestinationMask(dst));
            Write(dst, new Val(WgslText.Swizzle(converted.Text, slots.Select(res.SourceComponent).ToList()), converted.Kind, slots.Count));
        }

        private static int ResinfoDimensionLanes(ResourceDimension dimension) => dimension switch
        {
            ResourceDimension.Texture1D => 1,
            ResourceDimension.Texture3D => 3,
            _ => 2,
        };

        /// <summary>resinfo_rcpFloat reciprocates width, height and depth only; array size and mip count stay as plain floats.</summary>
        private static string ReciprocalDimensions(string packed, int dimensionLanes)
        {
            string lanes = string.Join(", ", Enumerable.Range(0, 4).Select(lane => lane < dimensionLanes ? "true" : "false"));
            return $"select(vec4<f32>({packed}), vec4<f32>(1.0) / vec4<f32>({packed}), vec4<bool>({lanes}))";
        }

        private (string Name, ResourceInfo Info) BufferOf(Operand res)
        {
            int reg = res.ImmediateIndex0;
            if (res.Type == OperandType.ThreadGroupSharedMemory)
            {
                if (!_tgsm.TryGetValue(reg, out var shared))
                    throw new WgslUnsupportedException($"{_name}: g{reg} is used but not declared.");
                return (ThreadGroupMemoryName(reg), shared);
            }
            if (res.Type == OperandType.UnorderedAccessView)
            {
                if (!_uavs.TryGetValue(reg, out var uav))
                    throw new WgslUnsupportedException($"{_name}: u{reg} is used but not declared.");
                if (!uav.Raw && !uav.Structured)
                    throw new WgslUnsupportedException($"{_name}: u{reg} is a typed UAV, not a raw or structured buffer.");
                return (Binding(_map.UnorderedAccess, "u", reg).Name, uav);
            }
            if (res.Type != OperandType.Resource || !_resources.TryGetValue(reg, out var info) || !(info.Raw || info.Structured))
                throw new WgslUnsupportedException($"{_name}: {Sm5Disassembler.FormatOperand(res)} is not a raw or structured buffer.");
            return (Binding(_map.Resources, "t", reg).Name, info);
        }

        private string WordBase(Instruction ins, ResourceInfo info, int offsetOperand)
        {
            if (ins.Opcode is Opcode.LdRaw or Opcode.StoreRaw)
                return $"({Read(ins.Operands[offsetOperand], 1, Kind.U32).Text} >> 2u)";
            var index = Read(ins.Operands[offsetOperand], 1, Kind.U32).Text;
            var byteOffset = Read(ins.Operands[offsetOperand + 1], 1, Kind.U32).Text;
            return StructuredWordIndex(index, byteOffset, info);
        }

        private static string StructuredWordIndex(string elementIndex, string byteOffset, ResourceInfo info) =>
            $"({elementIndex} * {info.StrideBytes / 4}u + ({byteOffset} >> 2u))";

        private void EmitBufferLoad(Instruction ins)
        {
            var dst = ins.Operands[0];
            var res = ins.Operands[^1];
            var (name, info) = BufferOf(res);
            string wordBase = Bind(new Val(WordBase(ins, info, 1), Kind.U32, 1)).Text;
            var slots = Slots(DestinationMask(dst));
            bool atomic = IsAtomicBuffer(res);
            var words = slots.Select(s => atomic
                ? $"atomicLoad(&{name}[{wordBase} + {res.SourceComponent(s)}u])"
                : $"{name}[{wordBase} + {res.SourceComponent(s)}u]").ToList();
            Write(dst, new Val(WgslText.Construct(Kind.U32, words), Kind.U32, words.Count));
        }

        private void EmitBufferStore(Instruction ins)
        {
            var dst = ins.Operands[0];
            var (name, info) = BufferOf(dst);
            if (dst.Type is not (OperandType.UnorderedAccessView or OperandType.ThreadGroupSharedMemory))
                throw new WgslUnsupportedException($"{_name}: stores to {Sm5Disassembler.FormatOperand(dst)} are not supported.");
            int srcOperand = ins.Opcode == Opcode.StoreRaw ? 2 : 3;
            string wordBase = Bind(new Val(WordBase(ins, info, 1), Kind.U32, 1)).Text;
            var slots = Slots(DestinationMask(dst));
            var src = Read(ins.Operands[srcOperand], DestinationMask(dst), Kind.U32);
            if (slots.Count > 1) src = Bind(src);
            bool atomic = IsAtomicBuffer(dst);
            for (int k = 0; k < slots.Count; k++)
            {
                string value = slots.Count == 1 ? src.Text : $"{src.Text}.{"xyzw"[k]}";
                Line(atomic ? $"atomicStore(&{name}[{wordBase} + {k}u], {value});" : $"{name}[{wordBase} + {k}u] = {value};");
            }
        }

        private void EmitBufferInfo(Instruction ins)
        {
            var dst = ins.Operands[0];
            var (name, info) = BufferOf(ins.Operands[1]);
            string size = info.Raw ? $"(arrayLength(&{name}) * 4u)" : $"(arrayLength(&{name}) / {info.StrideBytes / 4}u)";
            int width = Slots(DestinationMask(dst)).Count;
            var parts = new List<string> { size };
            parts.AddRange(Enumerable.Repeat("0u", width - 1));
            Write(dst, new Val(WgslText.Construct(Kind.U32, parts), Kind.U32, width));
        }
    }
}
