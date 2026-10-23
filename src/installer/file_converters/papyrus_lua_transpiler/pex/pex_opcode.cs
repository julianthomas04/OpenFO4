// Papyrus VM opcode table. Cross-referenced against:
//   - Caprica/Caprica/pex/PexOpCode.h
//   - Champollion/Pex/OpCode.hpp
//   - UESP wiki on Skyrim_Mod:Compiled_Script_File_Format (plus FO4 deltas)
//
// Arity encoding:
//   Fixed N               exactly N operands
//   VarArgs(N)            N fixed operands, then one int operand holding the
//                         variadic arg count K, then K more operands.
//                         Total operands read = N + 1 + K.

namespace AssetConverter.PapyrusLua.Pex;

public enum OpCode : byte
{
    // Common opcodes (present in all Papyrus versions 3.1+)
    Nop = 0,
    IAdd = 1,
    FAdd = 2,
    ISub = 3,
    FSub = 4,
    IMul = 5,
    FMul = 6,
    IDiv = 7,
    FDiv = 8,
    IMod = 9,
    Not = 10,
    INeg = 11,
    FNeg = 12,
    Assign = 13,
    Cast = 14,
    CmpEq = 15,
    CmpLt = 16,
    CmpLte = 17,
    CmpGt = 18,
    CmpGte = 19,
    Jmp = 20,
    JmpT = 21,
    JmpF = 22,
    CallMethod = 23,
    CallParent = 24,
    CallStatic = 25,
    Return = 26,
    StrCat = 27,
    PropGet = 28,
    PropSet = 29,
    ArrayCreate = 30,
    ArrayLength = 31,
    ArrayGetElement = 32,
    ArraySetElement = 33,
    ArrayFindElement = 34,
    ArrayRFindElement = 35,

    // FO4 / Papyrus 3.9 additions
    Is = 36,
    StructCreate = 37,
    StructGet = 38,
    StructSet = 39,
    ArrayFindStruct = 40,
    ArrayRFindStruct = 41,
    ArrayAdd = 42,
    ArrayInsert = 43,
    ArrayRemoveLast = 44,
    ArrayRemove = 45,
    ArrayClear = 46,

    // FO4 coroutine/guard opcodes
    LockGuards = 47,
    UnlockGuards = 48,
    TryLockGuards = 49,
}

public enum OpCodeArity
{
    Fixed,
    VarArgs,
}

public readonly record struct OpCodeSpec(
    OpCode Code,
    string Mnemonic,
    int FixedOperands,
    OpCodeArity Arity);

public static class OpCodeTable
{
    // One spec per opcode, indexed by (int)OpCode. Null means unknown/unused.
    public static readonly OpCodeSpec[] Specs = BuildTable();

    public static OpCodeSpec Get(OpCode op)
    {
        int idx = (int)op;
        if (idx < 0 || idx >= Specs.Length)
            throw new InvalidDataException($"Unknown opcode: 0x{idx:X2}");
        var s = Specs[idx];
        if (s.Mnemonic == null)
            throw new InvalidDataException($"Unhandled opcode: 0x{idx:X2}");
        return s;
    }

    static OpCodeSpec[] BuildTable()
    {
        var t = new OpCodeSpec[256];

        // dest/arg operand counts below follow Caprica's PexOpCode arity declarations.
        // "dest" counts as an operand. Most binary ops are (dest, lhs, rhs) = 3.
        Set(t, OpCode.Nop, "nop", 0);
        Set(t, OpCode.IAdd, "iadd", 3);
        Set(t, OpCode.FAdd, "fadd", 3);
        Set(t, OpCode.ISub, "isub", 3);
        Set(t, OpCode.FSub, "fsub", 3);
        Set(t, OpCode.IMul, "imul", 3);
        Set(t, OpCode.FMul, "fmul", 3);
        Set(t, OpCode.IDiv, "idiv", 3);
        Set(t, OpCode.FDiv, "fdiv", 3);
        Set(t, OpCode.IMod, "imod", 3);
        Set(t, OpCode.Not, "not", 2);   // (dest, src)
        Set(t, OpCode.INeg, "ineg", 2);
        Set(t, OpCode.FNeg, "fneg", 2);
        Set(t, OpCode.Assign, "assign", 2);   // (dest, src)
        Set(t, OpCode.Cast, "cast", 2);   // (dest, src) — target type is dest's declared type
        Set(t, OpCode.CmpEq, "cmp_eq", 3);
        Set(t, OpCode.CmpLt, "cmp_lt", 3);
        Set(t, OpCode.CmpLte, "cmp_lte", 3);
        Set(t, OpCode.CmpGt, "cmp_gt", 3);
        Set(t, OpCode.CmpGte, "cmp_gte", 3);
        Set(t, OpCode.Jmp, "jmp", 1);   // (int offset)
        Set(t, OpCode.JmpT, "jmpt", 2);   // (cond, offset)
        Set(t, OpCode.JmpF, "jmpf", 2);
        Set(t, OpCode.CallMethod, "callmethod", 3, OpCodeArity.VarArgs); // (method, self, dest) + argCount + args
        Set(t, OpCode.CallParent, "callparent", 2, OpCodeArity.VarArgs); // (method, dest) + argCount + args
        Set(t, OpCode.CallStatic, "callstatic", 3, OpCodeArity.VarArgs); // (object, method, dest) + argCount + args
        Set(t, OpCode.Return, "return", 1);   // (value or None)
        Set(t, OpCode.StrCat, "strcat", 3);   // (dest, lhs, rhs)
        Set(t, OpCode.PropGet, "propget", 3);   // (property, self, dest)
        Set(t, OpCode.PropSet, "propset", 3);   // (property, self, value)
        Set(t, OpCode.ArrayCreate, "array_create", 2);   // (dest, size)
        Set(t, OpCode.ArrayLength, "array_length", 2);   // (dest, array)
        Set(t, OpCode.ArrayGetElement, "array_getelement", 3);   // (dest, array, index)
        Set(t, OpCode.ArraySetElement, "array_setelement", 3);   // (array, index, value)
        Set(t, OpCode.ArrayFindElement, "array_findelement", 4);   // (array, dest, value, startIdx)
        Set(t, OpCode.ArrayRFindElement, "array_rfindelement", 4);

        // FO4 additions
        Set(t, OpCode.Is, "is", 3);   // (dest, value, typeName)
        Set(t, OpCode.StructCreate, "struct_create", 1);   // (dest)
        Set(t, OpCode.StructGet, "struct_get", 3);   // (dest, struct, memberName)
        Set(t, OpCode.StructSet, "struct_set", 3);   // (struct, memberName, value)
        Set(t, OpCode.ArrayFindStruct, "array_findstruct", 5);   // (array, dest, memberName, value, startIdx)
        Set(t, OpCode.ArrayRFindStruct, "array_rfindstruct", 5);
        Set(t, OpCode.ArrayAdd, "array_add", 3);   // (array, value, count)
        Set(t, OpCode.ArrayInsert, "array_insert", 3);   // (array, value, index)
        Set(t, OpCode.ArrayRemoveLast, "array_removelast", 1);
        Set(t, OpCode.ArrayRemove, "array_remove", 3);
        Set(t, OpCode.ArrayClear, "array_clear", 1);
        Set(t, OpCode.LockGuards, "lock_guards", 0, OpCodeArity.VarArgs); // argCount + guard refs
        Set(t, OpCode.UnlockGuards, "unlock_guards", 0, OpCodeArity.VarArgs);
        Set(t, OpCode.TryLockGuards, "try_lock_guards", 1, OpCodeArity.VarArgs); // dest + argCount + guards

        return t;
    }

    static void Set(OpCodeSpec[] t, OpCode op, string mnemonic, int fixedOps, OpCodeArity arity = OpCodeArity.Fixed)
    {
        t[(int)op] = new OpCodeSpec(op, mnemonic, fixedOps, arity);
    }
}
