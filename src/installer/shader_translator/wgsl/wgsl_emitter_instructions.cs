using AssetConverter.ShaderTranslation.Dxbc;

namespace AssetConverter.ShaderTranslation.Wgsl
{
    internal sealed partial class WgslEmitter
    {
        private delegate string Formatter(string[] args, int width);

        private sealed record AluRule(Kind[] Sources, Kind Result, Formatter Format, bool MasksShiftCount = false);

        private static AluRule Rule(Kind result, Kind[] sources, Formatter format) => new(sources, result, format);

        private static AluRule ShiftRule(Kind result, Kind[] sources, Formatter format) => new(sources, result, format, MasksShiftCount: true);

        /// <summary>D3D uses the low five bits of a shift count; WGSL rejects constant counts of 32 and above at module creation.</summary>
        private const uint ShiftCountMask = 31;

        private const uint BitWidth = 32;

        /// <summary>Opcodes whose result WGSL cannot represent for some inputs (overflow, division by zero, out-of-range conversion).</summary>
        private static readonly HashSet<Opcode> OpcodesThatCanFailConstantEvaluation =
        [
            Opcode.Add, Opcode.Mul, Opcode.Mad, Opcode.Div, Opcode.Rcp, Opcode.Rsq, Opcode.Sqrt, Opcode.Exp, Opcode.Log, Opcode.Dp2, Opcode.Dp3,
            Opcode.Dp4, Opcode.Ftoi, Opcode.Ftou, Opcode.Iadd, Opcode.Ineg, Opcode.Imad, Opcode.Umad, Opcode.Imul, Opcode.Umul, Opcode.Ishl,
            Opcode.Ishr, Opcode.Ushr, Opcode.Udiv,
        ];

        private static readonly Kind[] F1 = [Kind.F32], F2 = [Kind.F32, Kind.F32], F3 = [Kind.F32, Kind.F32, Kind.F32];
        private static readonly Kind[] I1 = [Kind.I32], I2 = [Kind.I32, Kind.I32], I3 = [Kind.I32, Kind.I32, Kind.I32];
        private static readonly Kind[] U1 = [Kind.U32], U2 = [Kind.U32, Kind.U32], U3 = [Kind.U32, Kind.U32, Kind.U32];

        private static readonly Dictionary<Opcode, AluRule> AluRules = new()
        {
            [Opcode.Mov] = Rule(Kind.F32, F1, (a, _) => a[0]),
            [Opcode.Add] = Rule(Kind.F32, F2, (a, _) => $"({a[0]} + {a[1]})"),
            [Opcode.Mul] = Rule(Kind.F32, F2, (a, _) => $"({a[0]} * {a[1]})"),
            [Opcode.Div] = Rule(Kind.F32, F2, (a, _) => $"({a[0]} / {a[1]})"),
            [Opcode.Mad] = Rule(Kind.F32, F3, (a, _) => $"({a[0]} * {a[1]} + {a[2]})"),
            [Opcode.Sqrt] = Rule(Kind.F32, F1, (a, _) => $"sqrt({a[0]})"),
            [Opcode.Rsq] = Rule(Kind.F32, F1, (a, _) => $"inverseSqrt({a[0]})"),
            [Opcode.Rcp] = Rule(Kind.F32, F1, (a, w) => $"({WgslText.Splat(Kind.F32, w, "1.0")} / {a[0]})"),
            [Opcode.Exp] = Rule(Kind.F32, F1, (a, _) => $"exp2({a[0]})"),
            [Opcode.Log] = Rule(Kind.F32, F1, (a, _) => $"log2({a[0]})"),
            [Opcode.Frc] = Rule(Kind.F32, F1, (a, _) => $"fract({a[0]})"),
            [Opcode.RoundNe] = Rule(Kind.F32, F1, (a, _) => $"round({a[0]})"),
            [Opcode.RoundNi] = Rule(Kind.F32, F1, (a, _) => $"floor({a[0]})"),
            [Opcode.RoundPi] = Rule(Kind.F32, F1, (a, _) => $"ceil({a[0]})"),
            [Opcode.RoundZ] = Rule(Kind.F32, F1, (a, _) => $"trunc({a[0]})"),
            [Opcode.DerivRtx] = Rule(Kind.F32, F1, (a, _) => $"dpdx({a[0]})"),
            [Opcode.DerivRty] = Rule(Kind.F32, F1, (a, _) => $"dpdy({a[0]})"),
            [Opcode.DerivRtxCoarse] = Rule(Kind.F32, F1, (a, _) => $"dpdxCoarse({a[0]})"),
            [Opcode.DerivRtyCoarse] = Rule(Kind.F32, F1, (a, _) => $"dpdyCoarse({a[0]})"),
            [Opcode.DerivRtxFine] = Rule(Kind.F32, F1, (a, _) => $"dpdxFine({a[0]})"),
            [Opcode.DerivRtyFine] = Rule(Kind.F32, F1, (a, _) => $"dpdyFine({a[0]})"),

            [Opcode.Ftoi] = Rule(Kind.I32, F1, (a, w) => $"{WgslText.Type(Kind.I32, w)}({a[0]})"),
            [Opcode.Ftou] = Rule(Kind.U32, F1, (a, w) => $"{WgslText.Type(Kind.U32, w)}({a[0]})"),
            [Opcode.Itof] = Rule(Kind.F32, I1, (a, w) => $"{WgslText.Type(Kind.F32, w)}({a[0]})"),
            [Opcode.Utof] = Rule(Kind.F32, U1, (a, w) => $"{WgslText.Type(Kind.F32, w)}({a[0]})"),

            [Opcode.Eq] = Rule(Kind.Bool, F2, (a, _) => $"({a[0]} == {a[1]})"),
            [Opcode.Ne] = Rule(Kind.Bool, F2, (a, _) => $"({a[0]} != {a[1]})"),
            [Opcode.Lt] = Rule(Kind.Bool, F2, (a, _) => $"({a[0]} < {a[1]})"),
            [Opcode.Ge] = Rule(Kind.Bool, F2, (a, _) => $"({a[0]} >= {a[1]})"),
            [Opcode.Ieq] = Rule(Kind.Bool, I2, (a, _) => $"({a[0]} == {a[1]})"),
            [Opcode.Ine] = Rule(Kind.Bool, I2, (a, _) => $"({a[0]} != {a[1]})"),
            [Opcode.Ilt] = Rule(Kind.Bool, I2, (a, _) => $"({a[0]} < {a[1]})"),
            [Opcode.Ige] = Rule(Kind.Bool, I2, (a, _) => $"({a[0]} >= {a[1]})"),
            [Opcode.Ult] = Rule(Kind.Bool, U2, (a, _) => $"({a[0]} < {a[1]})"),
            [Opcode.Uge] = Rule(Kind.Bool, U2, (a, _) => $"({a[0]} >= {a[1]})"),

            [Opcode.And] = Rule(Kind.U32, U2, (a, _) => $"({a[0]} & {a[1]})"),
            [Opcode.Or] = Rule(Kind.U32, U2, (a, _) => $"({a[0]} | {a[1]})"),
            [Opcode.Xor] = Rule(Kind.U32, U2, (a, _) => $"({a[0]} ^ {a[1]})"),
            [Opcode.Not] = Rule(Kind.U32, U1, (a, _) => $"(~{a[0]})"),
            [Opcode.Iadd] = Rule(Kind.I32, I2, (a, _) => $"({a[0]} + {a[1]})"),
            [Opcode.Ineg] = Rule(Kind.I32, I1, (a, _) => $"(-{a[0]})"),
            [Opcode.Imad] = Rule(Kind.I32, I3, (a, _) => $"({a[0]} * {a[1]} + {a[2]})"),
            [Opcode.Umad] = Rule(Kind.U32, U3, (a, _) => $"({a[0]} * {a[1]} + {a[2]})"),
            [Opcode.Imax] = Rule(Kind.I32, I2, (a, _) => $"max({a[0]}, {a[1]})"),
            [Opcode.Imin] = Rule(Kind.I32, I2, (a, _) => $"min({a[0]}, {a[1]})"),
            [Opcode.Umax] = Rule(Kind.U32, U2, (a, _) => $"max({a[0]}, {a[1]})"),
            [Opcode.Umin] = Rule(Kind.U32, U2, (a, _) => $"min({a[0]}, {a[1]})"),
            [Opcode.Ishl] = ShiftRule(Kind.I32, [Kind.I32, Kind.U32], (a, _) => $"({a[0]} << {a[1]})"),
            [Opcode.Ishr] = ShiftRule(Kind.I32, [Kind.I32, Kind.U32], (a, _) => $"({a[0]} >> {a[1]})"),
            [Opcode.Ushr] = ShiftRule(Kind.U32, U2, (a, _) => $"({a[0]} >> {a[1]})"),
            [Opcode.Countbits] = Rule(Kind.U32, U1, (a, _) => $"countOneBits({a[0]})"),
            [Opcode.FirstbitLo] = Rule(Kind.U32, U1, (a, _) => $"firstTrailingBit({a[0]})"),
            [Opcode.Bfrev] = Rule(Kind.U32, U1, (a, _) => $"reverseBits({a[0]})"),
            [Opcode.FirstbitHi] = Rule(Kind.U32, U1, (a, w) =>
                $"select({WgslText.Splat(Kind.U32, w, "31u")} - firstLeadingBit({a[0]}), {WgslText.Splat(Kind.U32, w, "0xffffffffu")}, {a[0]} == {WgslText.Splat(Kind.U32, w, "0u")})"),
            [Opcode.FirstbitShi] = Rule(Kind.U32, I1, (a, w) =>
                $"select({WgslText.Splat(Kind.U32, w, "31u")} - bitcast<{WgslText.Type(Kind.U32, w)}>(firstLeadingBit({a[0]})), {WgslText.Splat(Kind.U32, w, "0xffffffffu")}, bitcast<{WgslText.Type(Kind.U32, w)}>(firstLeadingBit({a[0]})) == {WgslText.Splat(Kind.U32, w, "0xffffffffu")})"),
        };

        private void EmitInstruction(Instruction ins)
        {
            _opaqueImmediates = IsConstantFoldable(ins);
            switch (ins.Opcode)
            {
                case Opcode.If:
                    Line($"if ({Condition(ins)}) {{");
                    _indent++;
                    return;
                case Opcode.Else:
                    _indent--;
                    Line("} else {");
                    _indent++;
                    return;
                case Opcode.Endif:
                case Opcode.Endloop:
                    _indent--;
                    Line("}");
                    return;
                case Opcode.Loop:
                    Line("loop {");
                    _indent++;
                    return;
                case Opcode.Break: Line("break;"); return;
                case Opcode.Continue: Line("continue;"); return;
                case Opcode.Breakc: ConditionalStatement(ins, () => Line("break;")); return;
                case Opcode.Continuec: ConditionalStatement(ins, () => Line("continue;")); return;
                case Opcode.Retc:
                    _exitedConditionally = true;
                    ConditionalStatement(ins, EmitReturn);
                    return;
                case Opcode.Discard: ConditionalStatement(ins, () => Line($"{DiscardFlag} = true;")); return;
                case Opcode.Ret: EmitReturn(); return;
                case Opcode.Switch: BeginSwitch(ins); return;
                case Opcode.Case: AddClauseLabel(CaseLabel(ins)); return;
                case Opcode.Default: AddClauseLabel("default"); return;
                case Opcode.Endswitch: EndSwitch(); return;
                case Opcode.Nop: return;
                case Opcode.CustomData:
                    return;
                case Opcode.Sync:
                    EmitSync(ins);
                    return;
            }

            if (OpcodeInfo.IsDeclaration(ins.Opcode)) return;
            if (EmitSpecialAlu(ins)) return;
            if (AluRules.TryGetValue(ins.Opcode, out var rule))
            {
                EmitRule(ins, rule);
                return;
            }
            if (EmitResourceInstruction(ins) || EmitAtomic(ins)) return;
            throw new WgslUnsupportedException($"{_name}: instruction '{Sm5Disassembler.Format(ins)}' is not supported.");
        }

        private string Condition(Instruction ins)
        {
            var test = Read(ins.Operands[0], 1, Kind.U32);
            return ins.TestNonZero ? $"{test.Text} != 0u" : $"{test.Text} == 0u";
        }

        private void ConditionalStatement(Instruction ins, Action emitBody)
        {
            Line($"if ({Condition(ins)}) {{");
            _indent++;
            emitBody();
            _indent--;
            Line("}");
        }

        /// <summary>
        /// True when every source is a literal: naga then constant-evaluates the expression and rejects overflow, division by zero and
        /// the like at module creation, where D3D defines a result.
        /// </summary>
        private static bool IsConstantFoldable(Instruction ins)
        {
            if (!OpcodesThatCanFailConstantEvaluation.Contains(ins.Opcode)) return false;
            var sources = ins.Operands.Skip(OpcodeInfo.DestinationCount(ins.Opcode)).ToList();
            return sources.Count > 0 && sources.All(s => s.IsImmediate);
        }

        /// <summary>Stores the expression in a mutable variable so the compiler cannot evaluate it ahead of time.</summary>
        private Val Opaque(Val value)
        {
            string name = NewTemp();
            Line($"var {name} = {value.Text};");
            return value with { Text = name };
        }

        private void EmitRule(Instruction ins, AluRule rule)
        {
            var dst = ins.Operands[0];
            int mask = DestinationMask(dst);
            int width = Slots(mask).Count;
            var args = new string[rule.Sources.Length];
            for (int k = 0; k < args.Length; k++)
            {
                uint immediateMask = rule.MasksShiftCount && k == 1 ? ShiftCountMask : uint.MaxValue;
                args[k] = Read(ins.Operands[1 + k], mask, rule.Sources[k], immediateMask).Text;
            }
            var result = new Val(rule.Format(args, width), rule.Result, width);
            Write(dst, result, ins.Saturate && rule.Result == Kind.F32);
        }

        private bool EmitSpecialAlu(Instruction ins)
        {
            switch (ins.Opcode)
            {
                case Opcode.Min:
                case Opcode.Max:
                    EmitMinMax(ins);
                    return true;
                case Opcode.Movc:
                    EmitMovc(ins);
                    return true;
                case Opcode.Dp2: EmitDot(ins, 2); return true;
                case Opcode.Dp3: EmitDot(ins, 3); return true;
                case Opcode.Dp4: EmitDot(ins, 4); return true;
                case Opcode.Sincos: EmitSincos(ins); return true;
                case Opcode.Udiv: EmitUdiv(ins); return true;
                case Opcode.Imul: case Opcode.Umul: EmitMultiplyLow(ins); return true;
                case Opcode.F32ToF16: EmitComponentWise(ins, [Kind.F32], Kind.U32, a => $"(pack2x16float(vec2<f32>({a[0]}, 0.0)) & 0xffffu)"); return true;
                case Opcode.F16ToF32: EmitComponentWise(ins, [Kind.U32], Kind.F32, a => $"unpack2x16float({a[0]} & 0xffffu).x"); return true;
                case Opcode.Ubfe:
                    EmitComponentWise(ins, [Kind.U32, Kind.U32, Kind.U32], Kind.U32,
                        a => $"extractBits({a[2]}, {BitfieldOffset(a[1])}, {BitfieldWidth(a[0], a[1])})");
                    return true;
                case Opcode.Ibfe:
                    EmitComponentWise(ins, [Kind.U32, Kind.U32, Kind.I32], Kind.I32,
                        a => $"extractBits({a[2]}, {BitfieldOffset(a[1])}, {BitfieldWidth(a[0], a[1])})");
                    return true;
                case Opcode.Bfi:
                    EmitComponentWise(ins, [Kind.U32, Kind.U32, Kind.U32, Kind.U32], Kind.U32,
                        a => $"insertBits({a[3]}, {a[2]}, {BitfieldOffset(a[1])}, {BitfieldWidth(a[0], a[1])})");
                    return true;
                default:
                    return false;
            }
        }

        private static string BitfieldOffset(string offset) => $"({offset} & {ShiftCountMask}u)";

        /// <summary>D3D clamps the field so that it ends at bit 31; WGSL requires offset + count to stay within the word.</summary>
        private static string BitfieldWidth(string width, string offset) =>
            $"min(({width} & {ShiftCountMask}u), {BitWidth}u - {BitfieldOffset(offset)})";

        private void EmitMinMax(Instruction ins)
        {
            var dst = ins.Operands[0];
            int mask = DestinationMask(dst);
            int width = Slots(mask).Count;
            string fn = ins.Opcode == Opcode.Min ? "min" : "max";
            var a = Read(ins.Operands[1], mask, Kind.F32);
            var b = Read(ins.Operands[2], mask, Kind.F32);
            string text;
            if (_options.Nan == NanPolicy.D3dStrict)
            {
                a = Bind(a);
                b = Bind(b);
                text = $"select(select({fn}({a.Text}, {b.Text}), {a.Text}, {b.Text} != {b.Text}), {b.Text}, {a.Text} != {a.Text})";
            }
            else
            {
                text = $"{fn}({a.Text}, {b.Text})";
            }
            Write(dst, new Val(text, Kind.F32, width), ins.Saturate);
        }

        private void EmitMovc(Instruction ins)
        {
            var dst = ins.Operands[0];
            int mask = DestinationMask(dst);
            int width = Slots(mask).Count;
            var cond = Read(ins.Operands[1], mask, Kind.Bool);
            var ifTrue = Read(ins.Operands[2], mask, Kind.F32);
            var ifFalse = Read(ins.Operands[3], mask, Kind.F32);
            Write(dst, new Val($"select({ifFalse.Text}, {ifTrue.Text}, {cond.Text})", Kind.F32, width), ins.Saturate);
        }

        private void EmitDot(Instruction ins, int components)
        {
            var dst = ins.Operands[0];
            int width = Slots(DestinationMask(dst)).Count;
            int srcMask = SlotMaskOf(components);
            var a = Read(ins.Operands[1], srcMask, Kind.F32);
            var b = Read(ins.Operands[2], srcMask, Kind.F32);
            string dot = $"dot({a.Text}, {b.Text})";
            Write(dst, new Val(width == 1 ? dot : $"vec{width}<f32>({dot})", Kind.F32, width), ins.Saturate);
        }

        private void EmitSincos(Instruction ins)
        {
            var sinDst = ins.Operands[0];
            var cosDst = ins.Operands[1];
            if (sinDst.Type != OperandType.Null)
            {
                int mask = DestinationMask(sinDst);
                Write(sinDst, new Val($"sin({Read(ins.Operands[2], mask, Kind.F32).Text})", Kind.F32, Slots(mask).Count), ins.Saturate);
            }
            if (cosDst.Type != OperandType.Null)
            {
                int mask = DestinationMask(cosDst);
                Write(cosDst, new Val($"cos({Read(ins.Operands[2], mask, Kind.F32).Text})", Kind.F32, Slots(mask).Count), ins.Saturate);
            }
        }

        private void EmitUdiv(Instruction ins)
        {
            var quotient = ins.Operands[0];
            var remainder = ins.Operands[1];
            int mask = DestinationMask(quotient.Type != OperandType.Null ? quotient : remainder);
            int width = Slots(mask).Count;
            var a = Bind(Read(ins.Operands[2], mask, Kind.U32));
            var divisor = Read(ins.Operands[3], mask, Kind.U32);
            var b = ins.Operands[3].IsImmediate ? Opaque(divisor) : Bind(divisor);
            string zero = WgslText.Splat(Kind.U32, width, "0u");
            string ones = WgslText.Splat(Kind.U32, width, "0xffffffffu");
            Val GuardedDivision(string op) => new($"select({a.Text} {op} {b.Text}, {ones}, {b.Text} == {zero})", Kind.U32, width);
            if (quotient.Type != OperandType.Null)
                Write(quotient, GuardedDivision("/"));
            if (remainder.Type != OperandType.Null)
            {
                int remainderWidth = Slots(DestinationMask(remainder)).Count;
                if (remainderWidth != width)
                    throw new WgslUnsupportedException($"{_name}: udiv with differing destination masks is not supported.");
                Write(remainder, GuardedDivision("%"));
            }
        }

        private void EmitMultiplyLow(Instruction ins)
        {
            if (ins.Operands[0].Type != OperandType.Null)
                throw new WgslUnsupportedException($"{_name}: {OpcodeInfo.Name(ins.Opcode)} high-word result is not supported.");
            var dst = ins.Operands[1];
            int mask = DestinationMask(dst);
            var kind = ins.Opcode == Opcode.Imul ? Kind.I32 : Kind.U32;
            var a = Read(ins.Operands[2], mask, kind);
            var b = Read(ins.Operands[3], mask, kind);
            Write(dst, new Val($"({a.Text} * {b.Text})", kind, Slots(mask).Count));
        }

        private void EmitComponentWise(Instruction ins, Kind[] sources, Kind result, Func<string[], string> scalar)
        {
            var dst = ins.Operands[0];
            int mask = DestinationMask(dst);
            int width = Slots(mask).Count;
            var values = new Val[sources.Length];
            for (int k = 0; k < sources.Length; k++)
            {
                values[k] = Read(ins.Operands[1 + k], mask, sources[k]);
                if (width > 1) values[k] = Bind(values[k]);
            }
            var components = new List<string>();
            for (int c = 0; c < width; c++)
                components.Add(scalar(values.Select(v => width == 1 ? v.Text : $"{v.Text}.{"xyzw"[c]}").ToArray()));
            Write(dst, new Val(WgslText.Construct(result, components), result, width));
        }

        private void BeginSwitch(Instruction ins)
        {
            var selector = Read(ins.Operands[0], 1, Kind.I32);
            Line($"switch ({selector.Text}) {{");
            _indent++;
            _switches.Push(new SwitchState());
        }

        private static string CaseLabel(Instruction ins)
        {
            uint bits = ins.Operands[0].Immediate[0];
            return unchecked((int)bits).ToString(System.Globalization.CultureInfo.InvariantCulture);
        }

        private void AddClauseLabel(string selector)
        {
            var state = _switches.Peek();
            if (state.ClauseOpen && state.ClauseEmpty)
            {
                state.Selectors.Add(selector);
                _lines[state.HeaderLine] = ClauseHeader(state);
            }
            else
            {
                CloseClause(state);
                state.Selectors = [selector];
                state.ClauseIndent = _indent;
                state.HeaderLine = _lines.Count;
                state.ClauseOpen = true;
                state.ClauseEmpty = true;
                state.LastTerminated = false;
                _lines.Add(ClauseHeader(state));
                _indent++;
            }
            if (selector == "default") state.SawDefault = true;
        }

        private static string ClauseHeader(SwitchState state) =>
            new string(' ', state.ClauseIndent * 4)
            + (state.Selectors is ["default"] ? "default: {" : $"case {string.Join(", ", state.Selectors)}: {{");

        private void CloseClause(SwitchState state)
        {
            if (!state.ClauseOpen) return;
            if (!state.ClauseEmpty && !state.LastTerminated)
                throw new WgslUnsupportedException($"{_name}: switch clause falls through into the next clause.");
            _indent--;
            _lines.Add(new string(' ', _indent * 4) + "}");
            state.ClauseOpen = false;
        }

        private void EndSwitch()
        {
            var state = _switches.Peek();
            CloseClause(state);
            if (!state.SawDefault)
                _lines.Add(new string(' ', _indent * 4) + "default: { }");
            _switches.Pop();
            _indent--;
            Line("}");
        }
    }
}
