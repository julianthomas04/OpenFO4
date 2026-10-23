// Lowers PexInstruction sequences to Lua.
//
// Phase 5: Jmp/JmpT/JmpF, CallMethod, CallParent, CallStatic,
//          PropGet, PropSet, StrCat.
// Phase 6: Arrays (Create/Length/Get/Set/Find/Add/Insert/Remove/Clear),
//          Structs (Create/Get/Set), Is, Cast (runtime helper).
// Guard opcodes (LockGuards etc.) emit _papyrus.* stubs.

using System.Globalization;
using AssetConverter.PapyrusLua.Pex;

namespace AssetConverter.PapyrusLua.Lua;

public sealed class FunctionContext
{
    public PexObject Script { get; }
    public PexFunction Function { get; }

    // Case-insensitive: Papyrus is case-insensitive.
    readonly Dictionary<string, string> _nameMap =
        new(StringComparer.OrdinalIgnoreCase);
    readonly Dictionary<string, string> _typeMap =
        new(StringComparer.OrdinalIgnoreCase);

    public FunctionContext(PexObject obj, PexFunction fn)
    {
        Script = obj;
        Function = fn;
        _nameMap["self"] = "self";
        foreach (var p in fn.Parameters)
        {
            _nameMap[p.Name] = NameMangling.ToLocalIdentifier(p.Name);
            _typeMap[p.Name] = p.TypeName;
        }
        foreach (var l in fn.Locals)
        {
            _nameMap[l.Name] = NameMangling.ToLocalIdentifier(l.Name);
            _typeMap[l.Name] = l.TypeName;
        }
        foreach (var v in obj.Variables)
            _typeMap[v.Name] = v.TypeName;
    }

    public string Operand(PexValue v) => v switch
    {
        PexValue.None => "nil",
        PexValue.Bool b => b.Value ? "true" : "false",
        PexValue.IntValue i => i.Value.ToString(CultureInfo.InvariantCulture),
        PexValue.FloatValue f => LuaNumberFormat.FloatLiteral(f.Value),
        PexValue.Str s => NameMangling.ToLuaStringLiteral(s.Value),
        PexValue.Identifier id => Ident(id.Name),
        _ => "nil",
    };

    public string Ident(string name)
    {
        if (_nameMap.TryGetValue(name, out var mapped))
            return mapped;
        return $"self[{NameMangling.ToLuaStringLiteral(name)}]";
    }

    public string DestIdent(PexValue v)
    {
        if (v is PexValue.Identifier id)
            return Ident(id.Name);
        throw new InvalidDataException($"expected identifier operand, got {v}");
    }

    public string? GetDeclaredType(PexValue v)
    {
        if (v is PexValue.Identifier id && _typeMap.TryGetValue(id.Name, out var t))
            return t;
        return null;
    }

}

public static class OpcodeLowering
{
    // Entry point: emit labels + lower all instructions in a function body.
    public static void LowerFunction(LuaWriter w, FunctionContext ctx, PexInstruction[] instrs)
    {
        var jumpTargets = FindJumpTargets(instrs);
        for (int i = 0; i < instrs.Length; i++)
        {
            if (jumpTargets.Contains(i))
                w.WriteLine($"::L_{i}::");
            Lower(w, ctx, instrs, i);
        }
        // No sentinel label: past-end jumps are emitted as inline returns (do return end).
    }

    // Discover which instruction indices are jump targets.
    // Offsets are relative to the CURRENT instruction (target = i + offset).
    // Past-end targets are NOT added to the set (they become inline returns in Lower).
    static HashSet<int> FindJumpTargets(PexInstruction[] instrs)
    {
        var targets = new HashSet<int>();
        for (int i = 0; i < instrs.Length; i++)
        {
            var op = instrs[i].Operands;
            switch (instrs[i].OpCode)
            {
                case OpCode.Jmp:
                    {
                        int t = i + AsInt(op[0]);
                        if (t < instrs.Length) targets.Add(t);
                        break;
                    }
                case OpCode.JmpT:
                case OpCode.JmpF:
                    {
                        int t = i + AsInt(op[1]);
                        if (t < instrs.Length) targets.Add(t);
                        break;
                    }
            }
        }
        return targets;
    }

    static void Lower(LuaWriter w, FunctionContext ctx, PexInstruction[] instrs, int i)
    {
        var instr = instrs[i];
        bool isLast = i == instrs.Length - 1;
        var op = instr.Operands;

        switch (instr.OpCode)
        {
            case OpCode.Nop:
                return;

            // --- arithmetic -------------------------------------------------
            case OpCode.IAdd:
            case OpCode.FAdd:
                Bin(w, ctx, op, "+"); return;
            case OpCode.ISub:
            case OpCode.FSub:
                Bin(w, ctx, op, "-"); return;
            case OpCode.IMul:
            case OpCode.FMul:
                Bin(w, ctx, op, "*"); return;
            case OpCode.FDiv:
                Bin(w, ctx, op, "/"); return;
            case OpCode.IDiv:
                // Truncate toward zero (C semantics), not floor (Lua semantics).
                w.WriteLine(
                    $"{ctx.DestIdent(op[0])} = (function(a,b) " +
                    $"local q = a/b; " +
                    $"return (q >= 0) and math.floor(q) or math.ceil(q) " +
                    $"end)({ctx.Operand(op[1])}, {ctx.Operand(op[2])})");
                return;
            case OpCode.IMod:
                // Sign follows dividend (C semantics), not divisor (Lua).
                w.WriteLine(
                    $"{ctx.DestIdent(op[0])} = (function(a,b) " +
                    $"local q = a/b; " +
                    $"local t = (q >= 0) and math.floor(q) or math.ceil(q); " +
                    $"return a - t*b " +
                    $"end)({ctx.Operand(op[1])}, {ctx.Operand(op[2])})");
                return;

            // --- unary ------------------------------------------------------
            case OpCode.Not:
                w.WriteLine($"{ctx.DestIdent(op[0])} = not ({ctx.Operand(op[1])})");
                return;
            case OpCode.INeg:
            case OpCode.FNeg:
                w.WriteLine($"{ctx.DestIdent(op[0])} = -({ctx.Operand(op[1])})");
                return;

            // --- comparison -------------------------------------------------
            case OpCode.CmpEq: Bin(w, ctx, op, "=="); return;
            case OpCode.CmpLt: Bin(w, ctx, op, "<"); return;
            case OpCode.CmpLte: Bin(w, ctx, op, "<="); return;
            case OpCode.CmpGt: Bin(w, ctx, op, ">"); return;
            case OpCode.CmpGte: Bin(w, ctx, op, ">="); return;

            // --- move / cast ------------------------------------------------
            case OpCode.Assign:
                w.WriteLine($"{ctx.DestIdent(op[0])} = {ctx.Operand(op[1])}");
                return;
            case OpCode.Cast:
                {
                    string destLua = ctx.DestIdent(op[0]);
                    string srcLua = ctx.Operand(op[1]);
                    string? type = ctx.GetDeclaredType(op[0]);
                    if (type != null)
                        w.WriteLine($"{destLua} = _papyrus.cast({srcLua}, {NameMangling.ToLuaStringLiteral(type)})");
                    else
                        w.WriteLine($"{destLua} = {srcLua}  -- cast");
                    return;
                }

            // --- control flow -----------------------------------------------
            case OpCode.Return:
                if (isLast)
                    w.WriteLine($"return {ctx.Operand(op[0])}");
                else
                    w.WriteLine($"do return {ctx.Operand(op[0])} end");
                return;

            case OpCode.Jmp:
                {
                    int raw = i + AsInt(op[0]);
                    if (raw >= instrs.Length)
                        w.WriteLine("do return end");
                    else
                        w.WriteLine($"goto L_{raw}");
                    return;
                }
            case OpCode.JmpT:
                {
                    int raw = i + AsInt(op[1]);
                    string cond = ctx.Operand(op[0]);
                    if (raw >= instrs.Length)
                        w.WriteLine($"if ({cond}) then return end");
                    else
                        w.WriteLine($"if ({cond}) then goto L_{raw} end");
                    return;
                }
            case OpCode.JmpF:
                {
                    int raw = i + AsInt(op[1]);
                    string cond = ctx.Operand(op[0]);
                    if (raw >= instrs.Length)
                        w.WriteLine($"if not ({cond}) then return end");
                    else
                        w.WriteLine($"if not ({cond}) then goto L_{raw} end");
                    return;
                }

            // --- function calls ---------------------------------------------
            case OpCode.CallMethod:
                {
                    // op: [methodName, receiver, dest, argCount, arg0, ...]
                    string method = AsStr(op[0]);
                    string receiver = ctx.Operand(op[1]);
                    int argCount = AsInt(op[3]);
                    string argList = BuildArgList(ctx, op, 4, argCount);

                    string callExpr = MakeMethodCall(method, receiver, argList);
                    EmitCall(w, ctx, op[2], callExpr);
                    return;
                }
            case OpCode.CallParent:
                {
                    // op: [methodName, dest, argCount, arg0, ...]
                    string method = AsStr(op[0]);
                    int argCount = AsInt(op[2]);
                    string argList = BuildArgList(ctx, op, 3, argCount);
                    string allArgs = argCount > 0 ? $"self, {argList}" : "self";

                    string callExpr = $"{MemberRef("_papyrus.parent(self)", method)}({allArgs})";
                    EmitCall(w, ctx, op[1], callExpr);
                    return;
                }
            case OpCode.CallStatic:
                {
                    // op: [scriptName, methodName, dest, argCount, arg0, ...]
                    string scriptName = AsStr(op[0]);
                    string method = AsStr(op[1]);
                    int argCount = AsInt(op[3]);
                    string argList = BuildArgList(ctx, op, 4, argCount);

                    string scriptRef = $"_papyrus.scripts[{NameMangling.ToLuaStringLiteral(scriptName)}]";
                    string callExpr = $"{MemberRef(scriptRef, method)}({argList})";
                    EmitCall(w, ctx, op[2], callExpr);
                    return;
                }

            // --- property access --------------------------------------------
            case OpCode.PropGet:
                {
                    // op: [propName, receiver, dest]
                    string propName = AsStr(op[0]);
                    string receiver = ctx.Operand(op[1]);
                    string dest = ctx.DestIdent(op[2]);
                    w.WriteLine($"{dest} = {MakeMethodCall($"get_{propName}", receiver, "")}");
                    return;
                }
            case OpCode.PropSet:
                {
                    // op: [propName, receiver, value]
                    string propName = AsStr(op[0]);
                    string receiver = ctx.Operand(op[1]);
                    string value = ctx.Operand(op[2]);
                    w.WriteLine(MakeMethodCall($"set_{propName}", receiver, value));
                    return;
                }

            // --- string concat ----------------------------------------------
            case OpCode.StrCat:
                w.WriteLine(
                    $"{ctx.DestIdent(op[0])} = " +
                    $"tostring({ctx.Operand(op[1])}) .. tostring({ctx.Operand(op[2])})");
                return;

            // --- arrays -----------------------------------------------------
            case OpCode.ArrayCreate:
                {
                    // op: [dest, size] — creates a 0-initialised Papyrus array.
                    string dest = ctx.DestIdent(op[0]);
                    string size = ctx.Operand(op[1]);
                    w.WriteLine($"{dest} = _papyrus.array_create({size})");
                    return;
                }
            case OpCode.ArrayLength:
                {
                    // op: [dest, array]
                    w.WriteLine($"{ctx.DestIdent(op[0])} = #{ctx.Operand(op[1])}");
                    return;
                }
            case OpCode.ArrayGetElement:
                {
                    // op: [dest, array, index]  — Papyrus 0-based → Lua 1-based
                    string dest = ctx.DestIdent(op[0]);
                    string arr = ctx.Operand(op[1]);
                    string idx = ctx.Operand(op[2]);
                    w.WriteLine($"{dest} = {arr}[{idx} + 1]");
                    return;
                }
            case OpCode.ArraySetElement:
                {
                    // op: [array, index, value]
                    string arr = ctx.Operand(op[0]);
                    string idx = ctx.Operand(op[1]);
                    string value = ctx.Operand(op[2]);
                    w.WriteLine($"{arr}[{idx} + 1] = {value}");
                    return;
                }
            case OpCode.ArrayFindElement:
                {
                    // op: [array, dest, value, startIdx]  — returns 0-based or -1
                    string arr = ctx.Operand(op[0]);
                    string dest = ctx.DestIdent(op[1]);
                    string value = ctx.Operand(op[2]);
                    string startIdx = ctx.Operand(op[3]);
                    w.WriteLine($"{dest} = _papyrus.array_find({arr}, {value}, {startIdx})");
                    return;
                }
            case OpCode.ArrayRFindElement:
                {
                    // op: [array, dest, value, startIdx]
                    string arr = ctx.Operand(op[0]);
                    string dest = ctx.DestIdent(op[1]);
                    string value = ctx.Operand(op[2]);
                    string startIdx = ctx.Operand(op[3]);
                    w.WriteLine($"{dest} = _papyrus.array_rfind({arr}, {value}, {startIdx})");
                    return;
                }
            case OpCode.ArrayAdd:
                {
                    // op: [array, value, count] — append value count times
                    string arr = ctx.Operand(op[0]);
                    string value = ctx.Operand(op[1]);
                    string count = ctx.Operand(op[2]);
                    w.WriteLine($"_papyrus.array_add({arr}, {value}, {count})");
                    return;
                }
            case OpCode.ArrayInsert:
                {
                    // op: [array, value, index] — insert before 0-based index
                    string arr = ctx.Operand(op[0]);
                    string value = ctx.Operand(op[1]);
                    string idx = ctx.Operand(op[2]);
                    w.WriteLine($"table.insert({arr}, {idx} + 1, {value})");
                    return;
                }
            case OpCode.ArrayRemoveLast:
                {
                    // op: [array]
                    w.WriteLine($"table.remove({ctx.Operand(op[0])})");
                    return;
                }
            case OpCode.ArrayRemove:
                {
                    // op: [array, index, count]
                    string arr = ctx.Operand(op[0]);
                    string idx = ctx.Operand(op[1]);
                    string count = ctx.Operand(op[2]);
                    w.WriteLine($"_papyrus.array_remove({arr}, {idx}, {count})");
                    return;
                }
            case OpCode.ArrayClear:
                {
                    // op: [array]
                    w.WriteLine($"_papyrus.array_clear({ctx.Operand(op[0])})");
                    return;
                }
            case OpCode.ArrayFindStruct:
                {
                    // op: [array, dest, memberName, value, startIdx]
                    string arr = ctx.Operand(op[0]);
                    string dest = ctx.DestIdent(op[1]);
                    string member = ctx.Operand(op[2]);
                    string value = ctx.Operand(op[3]);
                    string startIdx = ctx.Operand(op[4]);
                    w.WriteLine($"{dest} = _papyrus.array_find_struct({arr}, {member}, {value}, {startIdx})");
                    return;
                }
            case OpCode.ArrayRFindStruct:
                {
                    // op: [array, dest, memberName, value, startIdx]
                    string arr = ctx.Operand(op[0]);
                    string dest = ctx.DestIdent(op[1]);
                    string member = ctx.Operand(op[2]);
                    string value = ctx.Operand(op[3]);
                    string startIdx = ctx.Operand(op[4]);
                    w.WriteLine($"{dest} = _papyrus.array_rfind_struct({arr}, {member}, {value}, {startIdx})");
                    return;
                }

            // --- structs ----------------------------------------------------
            case OpCode.StructCreate:
                {
                    // op: [dest] — type inferred from dest's declared type
                    string dest = ctx.DestIdent(op[0]);
                    string? type = ctx.GetDeclaredType(op[0]);
                    if (type != null)
                        w.WriteLine($"{dest} = _papyrus.struct_create({NameMangling.ToLuaStringLiteral(type)})");
                    else
                        w.WriteLine($"{dest} = {{}}  -- struct_create (type unknown)");
                    return;
                }
            case OpCode.StructGet:
                {
                    // op: [dest, struct, memberName]
                    string dest = ctx.DestIdent(op[0]);
                    string svar = ctx.Operand(op[1]);
                    string member = NameMangling.ToLuaStringLiteral(AsStr(op[2]));
                    w.WriteLine($"{dest} = {svar}[{member}]");
                    return;
                }
            case OpCode.StructSet:
                {
                    // op: [struct, memberName, value]
                    string svar = ctx.Operand(op[0]);
                    string member = NameMangling.ToLuaStringLiteral(AsStr(op[1]));
                    string value = ctx.Operand(op[2]);
                    w.WriteLine($"{svar}[{member}] = {value}");
                    return;
                }

            // --- type check -------------------------------------------------
            case OpCode.Is:
                {
                    // op: [dest, value, typeName]
                    string dest = ctx.DestIdent(op[0]);
                    string value = ctx.Operand(op[1]);
                    string type = NameMangling.ToLuaStringLiteral(AsStr(op[2]));
                    w.WriteLine($"{dest} = _papyrus.is_type({value}, {type})");
                    return;
                }

            // --- guard opcodes (FO4 coroutine) ------------------------------
            case OpCode.LockGuards:
                {
                    // VarArgs: [argCount, guard0, guard1, ...]
                    int count = AsInt(op[0]);
                    string guards = BuildArgList(ctx, op, 1, count);
                    w.WriteLine($"_papyrus.lock_guards({guards})");
                    return;
                }
            case OpCode.UnlockGuards:
                {
                    int count = AsInt(op[0]);
                    string guards = BuildArgList(ctx, op, 1, count);
                    w.WriteLine($"_papyrus.unlock_guards({guards})");
                    return;
                }
            case OpCode.TryLockGuards:
                {
                    // VarArgs: [dest, argCount, guard0, ...]
                    string dest = ctx.DestIdent(op[0]);
                    int count = AsInt(op[1]);
                    string guards = BuildArgList(ctx, op, 2, count);
                    w.WriteLine($"{dest} = _papyrus.try_lock_guards({guards})");
                    return;
                }

            default:
                w.WriteLine($"-- TODO: opcode {instr.OpCode} ({op.Length} operands)");
                return;
        }
    }

    // Emit `dest = callExpr` or bare `callExpr` for void results.
    static void EmitCall(LuaWriter w, FunctionContext ctx, PexValue dest, string callExpr)
    {
        if (IsVoidDest(dest))
            w.WriteLine(callExpr);
        else
            w.WriteLine($"{ctx.DestIdent(dest)} = {callExpr}");
    }

    // Build a comma-separated arg list from op[startIdx..startIdx+count-1].
    static string BuildArgList(FunctionContext ctx, PexValue[] op, int startIdx, int count)
    {
        if (count == 0) return "";
        return string.Join(", ", Enumerable.Range(startIdx, count).Select(j => ctx.Operand(op[j])));
    }

    // Emit `receiver:method(args)` or bracket form for non-identifier methods.
    static string MemberRef(string target, string name) =>
        NameMangling.IsValidLuaIdentifier(name)
            ? $"{target}.{name}"
            : $"{target}[{NameMangling.ToLuaStringLiteral(name)}]";

    static string MakeMethodCall(string method, string receiver, string argList)
    {
        if (NameMangling.IsValidLuaIdentifier(method))
            return $"{receiver}:{method}({argList})";
        // Receiver evaluated twice but receiver is always a local variable.
        string key = NameMangling.ToLuaStringLiteral(method);
        return argList.Length == 0
            ? $"{receiver}[{key}]({receiver})"
            : $"{receiver}[{key}]({receiver}, {argList})";
    }

    // True when dest is the Papyrus discard variable (void call result).
    static bool IsVoidDest(PexValue dest) =>
        dest is PexValue.None ||
        (dest is PexValue.Identifier id &&
         string.Equals(id.Name, "::NoneVar", StringComparison.OrdinalIgnoreCase));

    static int AsInt(PexValue v) =>
        v is PexValue.IntValue i ? i.Value
        : throw new InvalidDataException($"Expected int operand, got {v}");

    static string AsStr(PexValue v) => v switch
    {
        PexValue.Str s => s.Value,
        PexValue.Identifier id => id.Name,
        _ => throw new InvalidDataException($"Expected string operand, got {v}"),
    };

    static void Bin(LuaWriter w, FunctionContext ctx, PexValue[] op, string luaOp) =>
        w.WriteLine($"{ctx.DestIdent(op[0])} = {ctx.Operand(op[1])} {luaOp} {ctx.Operand(op[2])}");
}
