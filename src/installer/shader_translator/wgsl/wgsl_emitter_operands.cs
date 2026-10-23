using System.Globalization;
using AssetConverter.ShaderTranslation.Dxbc;

namespace AssetConverter.ShaderTranslation.Wgsl
{
    internal sealed partial class WgslEmitter
    {
        private static List<int> Slots(int slotMask) => Enumerable.Range(0, 4).Where(s => (slotMask >> s & 1) != 0).ToList();

        private static int DestinationMask(Operand dst) =>
            dst.ComponentCount != 4 ? (dst.Type == OperandType.Null ? 0xF : 0x1) : dst.Mode == SelectionMode.Mask ? dst.Mask : 0xF;

        private static int SlotMaskOf(int count) => (1 << count) - 1;

        private Val Read(Operand op, int slotMask, Kind want, uint immediateMask = uint.MaxValue)
        {
            var slots = Slots(slotMask);
            if (IsComputeInput(op.Type))
                return ReadComputeInput(op, slots, want);
            if (op.IsImmediate)
            {
                var literal = ApplyModifier(ReadImmediate(op, slots, want, immediateMask), op.Modifier);
                return _opaqueImmediates ? Opaque(literal) : literal;
            }

            string text;
            if (op.ComponentCount == 4)
            {
                text = WgslText.Swizzle(RegisterText(op), slots.Select(op.SourceComponent).ToList());
            }
            else
            {
                string scalar = RegisterText(op);
                text = slots.Count == 1 ? scalar : $"vec{slots.Count}<f32>({scalar})";
            }
            var value = new Val(text, Kind.F32, slots.Count);
            if (want == Kind.F32)
                return ApplyModifier(value, op.Modifier);
            if (want == Kind.Bool)
                return ToBool(value);
            return ApplyModifier(Convert(value, want), op.Modifier);
        }

        private Val ReadImmediate(Operand op, List<int> slots, Kind want, uint immediateMask)
        {
            var bits = slots.Select(s => op.Immediate[op.ComponentCount == 1 ? 0 : s] & immediateMask).ToList();
            var literals = bits.Select(b => Literal(b, want)).ToList();
            if (literals.Count > 1 && literals.Distinct().Count() == 1)
                return new Val(WgslText.Splat(want, literals.Count, literals[0]), want, literals.Count);
            return new Val(WgslText.Construct(want, literals), want, literals.Count);
        }

        private string Literal(uint bits, Kind kind)
        {
            switch (kind)
            {
                case Kind.U32: return WgslText.IntLiteral(bits, signed: false);
                case Kind.I32: return WgslText.IntLiteral(bits, signed: true);
                case Kind.Bool: return bits != 0 ? "true" : "false";
                default:
                    float f = BitConverter.UInt32BitsToSingle(bits);
                    if (f == 0 || float.IsNormal(f)) return WgslText.FloatLiteral(f);
                    _runtimeFloatBits.Add(bits);
                    return $"bitcast<f32>({RuntimeBitsName(bits)})";
            }
        }

        private static Val ApplyModifier(Val v, OperandModifier modifier)
        {
            if (modifier == OperandModifier.None || v.Kind == Kind.Bool) return v;
            string t = v.Text;
            if (modifier is OperandModifier.Absolute or OperandModifier.AbsoluteNegate)
                t = v.Kind == Kind.U32
                    ? $"bitcast<{WgslText.Type(Kind.U32, v.Width)}>(abs(bitcast<{WgslText.Type(Kind.I32, v.Width)}>({t})))"
                    : $"abs({t})";
            if (modifier is OperandModifier.Negate or OperandModifier.AbsoluteNegate)
                t = v.Kind == Kind.U32 ? $"(0u - {t})" : $"(-{t})";
            return v with { Text = t };
        }

        private static Val Convert(Val v, Kind to)
        {
            if (v.Kind == to) return v;
            switch (v.Kind, to)
            {
                case (Kind.Bool, Kind.U32):
                    return new Val($"select({WgslText.Splat(Kind.U32, v.Width, "0u")}, {WgslText.Splat(Kind.U32, v.Width, "0xffffffffu")}, {v.Text})", Kind.U32, v.Width);
                case (Kind.Bool, _):
                    return Convert(Convert(v, Kind.U32), to);
                case (_, Kind.Bool):
                    return ToBool(v);
                default:
                    return new Val($"bitcast<{WgslText.Type(to, v.Width)}>({v.Text})", to, v.Width);
            }
        }

        private static Val ToBool(Val v)
        {
            if (v.Kind == Kind.Bool) return v;
            var u = Convert(v, Kind.U32);
            return new Val($"({u.Text} != {WgslText.Splat(Kind.U32, u.Width, "0u")})", Kind.Bool, u.Width);
        }

        private string RegisterText(Operand op)
        {
            switch (op.Type)
            {
                case OperandType.Temp:
                    return $"r{op.ImmediateIndex0}";
                case OperandType.Input:
                    RequireSingleIndex(op);
                    return $"i.v{op.ImmediateIndex0}";
                case OperandType.Output:
                    RequireSingleIndex(op);
                    return $"o.o{op.ImmediateIndex0}";
                case OperandType.ConstantBuffer:
                    return $"{Binding(_map.ConstantBuffers, "cb", op.ImmediateIndex0).Name}[{IndexExpression(op.Indices[1])}]";
                case OperandType.IndexableTemp:
                    return $"x{op.ImmediateIndex0}[{IndexExpression(op.Indices[1])}]";
                case OperandType.ImmediateConstantBuffer:
                    return $"bitcast<vec4<f32>>({IcbName}[{IndexExpression(op.Indices[0])}])";
                case OperandType.OutputDepth:
                case OperandType.OutputDepthGreaterEqual:
                case OperandType.OutputDepthLessEqual:
                    return "o.oDepth";
                case OperandType.OutputCoverageMask:
                    return "bitcast<f32>(o.oMask)";
                default:
                    throw new WgslUnsupportedException($"{_name}: operand {Sm5Disassembler.FormatOperand(op)} ({op.Type}) is not supported.");
            }
        }

        private void RequireSingleIndex(Operand op)
        {
            if (op.Indices.Count != 1 || op.Indices[0].Relative != null)
                throw new WgslUnsupportedException($"{_name}: indexed register {Sm5Disassembler.FormatOperand(op)} is not supported.");
        }

        private string IndexExpression(OperandIndex index)
        {
            if (index.Relative == null) return index.Immediate.ToString(CultureInfo.InvariantCulture);
            var rel = Read(index.Relative, 1, Kind.U32);
            return index.Immediate == 0 ? rel.Text : $"({rel.Text} + {index.Immediate}u)";
        }

        /// <summary>Stores <paramref name="value"/> (one component per destination mask bit) into the destination.</summary>
        private void Write(Operand dst, Val value, bool saturate = false)
        {
            if (dst.Type == OperandType.Null) return;
            value = Convert(value, Kind.F32);
            if (saturate)
                value = Saturate(value);

            string target = WriteTarget(dst);
            if (dst.Type is OperandType.OutputDepth or OperandType.OutputDepthGreaterEqual or OperandType.OutputDepthLessEqual)
            {
                Line($"{target} = {value.Text};");
                return;
            }
            if (dst.Type == OperandType.OutputCoverageMask)
            {
                Line($"o.oMask = bitcast<u32>({value.Text});");
                return;
            }

            var slots = Slots(DestinationMask(dst));
            if (slots.Count != value.Width)
                throw new InvalidOperationException($"{_name}: writing {value.Width} components to mask of {slots.Count}.");
            if (slots.Count == 4)
            {
                Line($"{target} = {value.Text};");
            }
            else if (slots.Count == 1)
            {
                Line($"{target}.{"xyzw"[slots[0]]} = {value.Text};");
            }
            else
            {
                var bound = Bind(value);
                for (int k = 0; k < slots.Count; k++)
                    Line($"{target}.{"xyzw"[slots[k]]} = {bound.Text}.{"xyzw"[k]};");
            }
        }

        /// <summary>Clamps to [0, 1]; under the strict NaN policy a NaN becomes 0 as in D3D (WGSL clamp leaves NaN unspecified).</summary>
        private Val Saturate(Val value)
        {
            string zero = WgslText.Splat(Kind.F32, value.Width, "0.0");
            string one = WgslText.Splat(Kind.F32, value.Width, "1.0");
            if (_options.Nan != NanPolicy.D3dStrict)
                return value with { Text = $"clamp({value.Text}, {zero}, {one})" };
            value = Bind(value);
            return value with { Text = $"select(clamp({value.Text}, {zero}, {one}), {zero}, {value.Text} != {value.Text})" };
        }

        private string WriteTarget(Operand dst) => dst.Type switch
        {
            OperandType.Temp => $"r{dst.ImmediateIndex0}",
            OperandType.Output => OutputRegister(dst),
            OperandType.IndexableTemp => $"x{dst.ImmediateIndex0}[{IndexExpression(dst.Indices[1])}]",
            OperandType.OutputDepth or OperandType.OutputDepthGreaterEqual or OperandType.OutputDepthLessEqual => "o.oDepth",
            OperandType.OutputCoverageMask => "o.oMask",
            _ => throw new WgslUnsupportedException($"{_name}: destination {Sm5Disassembler.FormatOperand(dst)} is not supported."),
        };

        private string OutputRegister(Operand dst)
        {
            RequireSingleIndex(dst);
            return $"o.o{dst.ImmediateIndex0}";
        }
    }
}
