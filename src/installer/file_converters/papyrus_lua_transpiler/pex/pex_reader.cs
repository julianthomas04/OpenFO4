// Binary deserializer for Papyrus .pex (v3.9 / Fallout 4).
//
// The format is a single-pass, length-prefixed binary layout. See
// pex_file.cs for the in-memory model
// for the field-by-field annotated layout. No seeking required.
//
// Endianness note: Skyrim .pex stored some integers big-endian, but
// every integer type in the Fallout 4 (major=3, minor=9) variant is
// little-endian. We assert the version in the header and then use
// BinaryReader's default LE reads throughout.

using System.Text;

namespace AssetConverter.PapyrusLua.Pex;

public static class PexReader
{
    const uint ExpectedMagic = 0xFA57C0DE;
    const ushort GameIdFO4 = 2;

    public static PexFile Read(string path)
    {
        using var fs = File.OpenRead(path);
        using var br = new BinaryReader(fs, Encoding.UTF8, leaveOpen: false);
        return Read(br, path);
    }

    public static PexFile Read(BinaryReader br, string diagPath)
    {
        var header = ReadHeader(br, diagPath);
        DebugLog(br, "after header");
        var strings = ReadStringTable(br);
        DebugLog(br, $"after string table ({strings.Length} strings)");
        var debug = ReadDebugInfo(br, strings);
        DebugLog(br, "after debug info");
        var userFlags = ReadUserFlags(br, strings);
        DebugLog(br, $"after user flags ({userFlags.Length})");
        var objects = ReadObjects(br, strings);
        return new PexFile(header, strings, debug, userFlags, objects);
    }

    [System.Diagnostics.Conditional("PEX_TRACE")]
    static void DebugLog(BinaryReader br, string label)
    {
        Console.Error.WriteLine($"  @0x{br.BaseStream.Position:X4}: {label}");
    }

    // ---- header ---------------------------------------------------------

    static PexHeader ReadHeader(BinaryReader br, string diagPath)
    {
        uint magic = br.ReadUInt32();
        if (magic != ExpectedMagic)
            throw new InvalidDataException(
                $"{diagPath}: bad magic 0x{magic:X8}, expected 0x{ExpectedMagic:X8}");

        byte major = br.ReadByte();
        byte minor = br.ReadByte();
        ushort gameId = br.ReadUInt16();
        if (gameId != GameIdFO4)
            throw new InvalidDataException(
                $"{diagPath}: unsupported gameId {gameId} (only FO4=2 is supported)");
        ulong compileTime = br.ReadUInt64();
        string src = ReadUtf8LengthPrefixed(br);
        string user = ReadUtf8LengthPrefixed(br);
        string machine = ReadUtf8LengthPrefixed(br);

        return new PexHeader(magic, major, minor, gameId, compileTime, src, user, machine);
    }

    // ---- string table ---------------------------------------------------

    static string[] ReadStringTable(BinaryReader br)
    {
        ushort count = br.ReadUInt16();
        var strings = new string[count];
        for (int i = 0; i < count; i++)
            strings[i] = ReadUtf8LengthPrefixed(br);
        return strings;
    }

    // ---- debug info (optional) -----------------------------------------

    static PexDebugInfo? ReadDebugInfo(BinaryReader br, string[] strings)
    {
        byte flag = br.ReadByte();
        if (flag == 0) return null;

        ulong modTime = br.ReadUInt64();

        ushort funcCount = br.ReadUInt16();
        var funcs = new PexDebugFunction[funcCount];
        for (int i = 0; i < funcCount; i++)
        {
            string objName = ReadStringRef(br, strings);
            string stateName = ReadStringRef(br, strings);
            string funcName = ReadStringRef(br, strings);
            byte funcType = br.ReadByte();
            ushort lineCount = br.ReadUInt16();
            var lines = new ushort[lineCount];
            for (int j = 0; j < lineCount; j++)
                lines[j] = br.ReadUInt16();
            funcs[i] = new PexDebugFunction(objName, stateName, funcName, funcType, lines);
        }

        // FO4+ adds property groups and struct orders to the debug info.
        ushort groupCount = br.ReadUInt16();
        var groups = new PexPropertyGroup[groupCount];
        for (int i = 0; i < groupCount; i++)
        {
            string objName = ReadStringRef(br, strings);
            string groupName = ReadStringRef(br, strings);
            string doc = ReadStringRef(br, strings);
            uint userFlags = br.ReadUInt32();
            ushort propCount = br.ReadUInt16();
            var props = new string[propCount];
            for (int j = 0; j < propCount; j++)
                props[j] = ReadStringRef(br, strings);
            groups[i] = new PexPropertyGroup(objName, groupName, doc, userFlags, props);
        }

        ushort orderCount = br.ReadUInt16();
        var orders = new PexStructOrder[orderCount];
        for (int i = 0; i < orderCount; i++)
        {
            string objName = ReadStringRef(br, strings);
            string ordName = ReadStringRef(br, strings);
            ushort memCount = br.ReadUInt16();
            var members = new string[memCount];
            for (int j = 0; j < memCount; j++)
                members[j] = ReadStringRef(br, strings);
            orders[i] = new PexStructOrder(objName, ordName, members);
        }

        return new PexDebugInfo(modTime, funcs, groups, orders);
    }

    // ---- user flags -----------------------------------------------------

    static PexUserFlag[] ReadUserFlags(BinaryReader br, string[] strings)
    {
        ushort count = br.ReadUInt16();
        var flags = new PexUserFlag[count];
        for (int i = 0; i < count; i++)
        {
            string name = ReadStringRef(br, strings);
            byte bit = br.ReadByte();
            flags[i] = new PexUserFlag(name, bit);
        }
        return flags;
    }

    // ---- objects --------------------------------------------------------

    static PexObject[] ReadObjects(BinaryReader br, string[] strings)
    {
        ushort count = br.ReadUInt16();
        var objs = new PexObject[count];
        for (int i = 0; i < count; i++)
            objs[i] = ReadObject(br, strings);
        return objs;
    }

    static PexObject ReadObject(BinaryReader br, string[] strings)
    {
        DebugLog(br, "  ReadObject start");
        string name = ReadStringRef(br, strings);
        DebugLog(br, $"  after name='{name}'");
        uint size = br.ReadUInt32();
        DebugLog(br, $"  after size={size}");
        // size counts the bytes of the remaining object definition (from here).
        // We don't rely on it for parsing — we parse structurally — but keep it
        // in the model for diagnostics / round-trip.

        string parent = ReadStringRef(br, strings);
        string doc = ReadStringRef(br, strings);
        // FO4 inserts a Const flag byte here that Skyrim did not have.
        byte constFlag = br.ReadByte();
        uint userFlags = br.ReadUInt32();
        string autoState = ReadStringRef(br, strings);

        // FO4+: struct table comes before variables.
        var structs = ReadStructs(br, strings);
        var variables = ReadVariables(br, strings);
        var properties = ReadProperties(br, strings);
        var states = ReadStates(br, strings);

        return new PexObject(name, size, parent, doc, constFlag, userFlags, autoState,
            structs, variables, properties, states);
    }

    // ---- structs (FO4+) -------------------------------------------------

    static PexStruct[] ReadStructs(BinaryReader br, string[] strings)
    {
        ushort count = br.ReadUInt16();
        var structs = new PexStruct[count];
        for (int i = 0; i < count; i++)
        {
            string name = ReadStringRef(br, strings);
            ushort memCount = br.ReadUInt16();
            var members = new PexStructMember[memCount];
            for (int j = 0; j < memCount; j++)
            {
                string mName = ReadStringRef(br, strings);
                string mType = ReadStringRef(br, strings);
                uint mFlags = br.ReadUInt32();
                var initVal = ReadValue(br, strings);
                byte constFlag = br.ReadByte();
                string mDoc = ReadStringRef(br, strings);
                members[j] = new PexStructMember(mName, mType, mFlags, initVal, constFlag, mDoc);
            }
            structs[i] = new PexStruct(name, members);
        }
        return structs;
    }

    // ---- variables ------------------------------------------------------

    static PexVariable[] ReadVariables(BinaryReader br, string[] strings)
    {
        ushort count = br.ReadUInt16();
        var vars = new PexVariable[count];
        for (int i = 0; i < count; i++)
        {
            string name = ReadStringRef(br, strings);
            string type = ReadStringRef(br, strings);
            uint userFlags = br.ReadUInt32();
            var initial = ReadValue(br, strings);
            byte constFlag = br.ReadByte();
            vars[i] = new PexVariable(name, type, userFlags, initial, constFlag);
        }
        return vars;
    }

    // ---- properties -----------------------------------------------------

    static PexProperty[] ReadProperties(BinaryReader br, string[] strings)
    {
        ushort count = br.ReadUInt16();
        var props = new PexProperty[count];
        for (int i = 0; i < count; i++)
        {
            string name = ReadStringRef(br, strings);
            string type = ReadStringRef(br, strings);
            string doc = ReadStringRef(br, strings);
            uint userFlags = br.ReadUInt32();
            byte flags = br.ReadByte();

            bool readable = (flags & 0x01) != 0;
            bool writable = (flags & 0x02) != 0;
            bool autoVar = (flags & 0x04) != 0;

            string autoVarName = "";
            PexFunction? readHandler = null;
            PexFunction? writeHandler = null;

            if (autoVar)
            {
                autoVarName = ReadStringRef(br, strings);
            }
            else
            {
                if (readable) readHandler = ReadFunction(br, strings, hasName: false);
                if (writable) writeHandler = ReadFunction(br, strings, hasName: false);
            }

            props[i] = new PexProperty(name, type, doc, userFlags, flags,
                autoVarName, readHandler, writeHandler);
        }
        return props;
    }

    // ---- states ---------------------------------------------------------

    static PexState[] ReadStates(BinaryReader br, string[] strings)
    {
        ushort count = br.ReadUInt16();
        var states = new PexState[count];
        for (int i = 0; i < count; i++)
        {
            string name = ReadStringRef(br, strings);
            ushort fnCount = br.ReadUInt16();
            var fns = new PexFunction[fnCount];
            for (int j = 0; j < fnCount; j++)
                fns[j] = ReadFunction(br, strings, hasName: true);
            states[i] = new PexState(name, fns);
        }
        return states;
    }

    // ---- function body --------------------------------------------------
    //
    // A function is the only structure whose name is sometimes inlined
    // (inside a state) and sometimes implicit (a property getter/setter
    // reads no name). The hasName flag selects between the two forms.

    static PexFunction ReadFunction(BinaryReader br, string[] strings, bool hasName)
    {
        string name = hasName ? ReadStringRef(br, strings) : "";
        string returnType = ReadStringRef(br, strings);
        string doc = ReadStringRef(br, strings);
        uint userFlags = br.ReadUInt32();
        byte flags = br.ReadByte();

        ushort paramCount = br.ReadUInt16();
        var parameters = new PexParameter[paramCount];
        for (int i = 0; i < paramCount; i++)
        {
            string pName = ReadStringRef(br, strings);
            string pType = ReadStringRef(br, strings);
            parameters[i] = new PexParameter(pName, pType);
        }

        ushort localCount = br.ReadUInt16();
        var locals = new PexLocal[localCount];
        for (int i = 0; i < localCount; i++)
        {
            string lName = ReadStringRef(br, strings);
            string lType = ReadStringRef(br, strings);
            locals[i] = new PexLocal(lName, lType);
        }

        ushort instrCount = br.ReadUInt16();
        var instrs = new PexInstruction[instrCount];
        for (int i = 0; i < instrCount; i++)
            instrs[i] = ReadInstruction(br, strings);

        return new PexFunction(name, returnType, doc, userFlags, flags,
            parameters, locals, instrs);
    }

    // ---- instruction ----------------------------------------------------

    static PexInstruction ReadInstruction(BinaryReader br, string[] strings)
    {
        byte opByte = br.ReadByte();
        var op = (OpCode)opByte;
        var spec = OpCodeTable.Get(op);

        // Read the fixed operands first.
        var fixedOps = new PexValue[spec.FixedOperands];
        for (int i = 0; i < spec.FixedOperands; i++)
            fixedOps[i] = ReadValue(br, strings);

        if (spec.Arity == OpCodeArity.Fixed)
            return new PexInstruction(op, fixedOps);

        // VarArgs: an int operand gives the count of trailing args.
        var argCountValue = ReadValue(br, strings);
        if (argCountValue is not PexValue.IntValue intCount)
            throw new InvalidDataException(
                $"Variadic opcode {spec.Mnemonic} expected int arg count, got {argCountValue}");
        int k = intCount.Value;
        if (k < 0 || k > 1024)
            throw new InvalidDataException(
                $"Variadic opcode {spec.Mnemonic} has implausible arg count {k}");

        var all = new PexValue[spec.FixedOperands + 1 + k];
        Array.Copy(fixedOps, all, spec.FixedOperands);
        all[spec.FixedOperands] = intCount;
        for (int i = 0; i < k; i++)
            all[spec.FixedOperands + 1 + i] = ReadValue(br, strings);
        return new PexInstruction(op, all);
    }

    // ---- value (tagged) -------------------------------------------------

    static PexValue ReadValue(BinaryReader br, string[] strings)
    {
        byte type = br.ReadByte();
        return type switch
        {
            0 => PexValue.None.Instance,
            1 => new PexValue.Identifier(ReadStringRef(br, strings)),
            2 => new PexValue.Str(ReadStringRef(br, strings)),
            3 => new PexValue.IntValue(br.ReadInt32()),
            4 => new PexValue.FloatValue(br.ReadSingle()),
            5 => new PexValue.Bool(br.ReadByte() != 0),
            _ => throw new InvalidDataException($"Unknown operand type byte: 0x{type:X2}"),
        };
    }

    // ---- primitives -----------------------------------------------------

    static string ReadUtf8LengthPrefixed(BinaryReader br)
    {
        ushort len = br.ReadUInt16();
        if (len == 0) return "";
        byte[] bytes = br.ReadBytes(len);
        if (bytes.Length != len)
            throw new EndOfStreamException($"Truncated string: wanted {len} bytes, got {bytes.Length}");
        // Caprica writes UTF-8; Bethesda's compiler writes the system ANSI code page (Windows-1252).
        try
        {
            return StrictUtf8.GetString(bytes);
        }
        catch (DecoderFallbackException)
        {
            return Windows1252.GetString(bytes);
        }
    }

    static readonly Encoding StrictUtf8 = new UTF8Encoding(encoderShouldEmitUTF8Identifier: false, throwOnInvalidBytes: true);
    static readonly Encoding Windows1252 = CodePagesEncodingProvider.Instance.GetEncoding(1252)!;

    static string ReadStringRef(BinaryReader br, string[] strings)
    {
        long before = br.BaseStream.Position;
        byte lo = br.ReadByte();
        byte hi = br.ReadByte();
        ushort idx = (ushort)(lo | (hi << 8));
        DebugLog(br, $"    StrRef @0x{before:X4}: bytes {lo:X2} {hi:X2} -> idx={idx}");
        if (idx >= strings.Length)
            throw new InvalidDataException(
                $"String ref {idx} out of range (table has {strings.Length})");
        return strings[idx];
    }
}
