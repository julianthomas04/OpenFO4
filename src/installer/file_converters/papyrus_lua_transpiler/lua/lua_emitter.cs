// Phase-3 scaffolding emitter: produces a .lua file per PexFile with the
// structural shape (script table, properties, states, function headers)
// but `-- TODO: N instructions` bodies instead of lowered opcodes.
//
// Every file produced here must be accepted by `luac -p`. If a file fails
// syntax checking, the bug is either here or in name_mangling.cs.

using System.Globalization;
using System.Text;
using AssetConverter.PapyrusLua.Pex;

namespace AssetConverter.PapyrusLua.Lua;

public static class LuaEmitter
{
    static readonly string[] GetterParams = { "self" };
    static readonly string[] SetterParams = { "self", "value" };

    public static string Emit(PexFile pex)
    {
        var w = new LuaWriter();
        w.WriteLine($"-- Transpiled from {pex.Header.SourceFileName}");
        w.WriteLine($"-- PEX compile time: {DateTimeOffset.FromUnixTimeSeconds((long)pex.Header.CompileTime):u}");
        w.WriteLine();

        foreach (var obj in pex.Objects)
            EmitObject(w, obj);

        return w.ToString();
    }

    static void EmitObject(LuaWriter w, PexObject obj)
    {
        // The script is always reachable as a module-level local named `__script`.
        // We avoid using the Papyrus script name as the local because it can
        // collide with a parameter/local in nested function emission, and
        // because `__script` keeps the generated code predictable for tooling.
        const string local = "__script";

        w.WriteLine($"-- ======================================================================");
        w.WriteLine($"-- Script: {obj.Name}" +
            (string.IsNullOrEmpty(obj.ParentClassName) ? "" : $" extends {obj.ParentClassName}"));
        w.WriteLine($"-- ======================================================================");
        w.WriteLine($"local {local} = {{}}");
        w.WriteLine($"{local}.__index = {local}");
        w.WriteLine($"{local}.__papyrus_name = {NameMangling.ToLuaStringLiteral(obj.Name)}");
        if (!string.IsNullOrEmpty(obj.ParentClassName))
            w.WriteLine($"{local}.__papyrus_extends = {NameMangling.ToLuaStringLiteral(obj.ParentClassName)}");
        if (obj.ConstFlag != 0)
            w.WriteLine($"{local}.__papyrus_const = true");
        if (!string.IsNullOrEmpty(obj.AutoStateName))
            w.WriteLine($"{local}.__papyrus_autoState = {NameMangling.ToLuaStringLiteral(obj.AutoStateName)}");
        w.WriteLine($"{local}.__states = {{}}");
        w.WriteLine($"{local}.__states[\"\"] = {local}");
        w.WriteLine();

        foreach (var s in obj.Structs)
            EmitStruct(w, local, s);

        if (obj.Variables.Length > 0)
        {
            w.WriteLine("-- ---- Script-scope variables ------------------------------------------");
            foreach (var v in obj.Variables)
                EmitVariable(w, local, v);
            w.WriteLine();
        }

        if (obj.Properties.Length > 0)
        {
            w.WriteLine("-- ---- Properties ------------------------------------------------------");
            foreach (var p in obj.Properties)
                EmitProperty(w, local, obj, p);
        }

        foreach (var st in obj.States)
            EmitState(w, local, obj, st);

        w.WriteLine($"return {local}");
        w.WriteLine();
    }

    static void EmitStruct(LuaWriter w, string script, PexStruct s)
    {
        string factoryName = $"__struct_{SanitizeForLocal(s.Name)}";
        w.WriteLine($"-- Struct {s.Name}");
        w.WriteLine($"function {script}.{factoryName}()");
        w.Indent();
        w.WriteLine("return {");
        w.Indent();
        foreach (var m in s.Members)
        {
            string key = NameMangling.ToLuaStringLiteral(m.Name);
            string init = LiteralOf(m.InitialValue);
            w.WriteLine($"[{key}] = {init},  -- {m.TypeName}");
        }
        w.Dedent();
        w.WriteLine("}");
        w.Dedent();
        w.WriteLine("end");
        w.WriteLine();
    }

    static void EmitVariable(LuaWriter w, string script, PexVariable v)
    {
        string key = NameMangling.ToLuaStringLiteral(v.Name);
        string init = LiteralOf(v.InitialValue);
        string trail = v.ConstFlag != 0 ? "  -- const" : "";
        w.WriteLine($"{script}[{key}] = {init}  -- {v.TypeName}{trail}");
    }

    static void EmitProperty(LuaWriter w, string script, PexObject obj, PexProperty p)
    {
        bool readable = (p.Flags & 0x01) != 0;
        bool writable = (p.Flags & 0x02) != 0;
        bool autoVar = (p.Flags & 0x04) != 0;

        var tags = new List<string>();
        if (readable) tags.Add("readable");
        if (writable) tags.Add("writable");
        if (autoVar) tags.Add("auto");
        w.WriteLine($"-- Property {p.Name} : {p.TypeName} [{string.Join(", ", tags)}]");

        if (autoVar)
        {
            string bkey = NameMangling.ToLuaStringLiteral(p.AutoVarName);
            EmitMethodOpen(w, script, $"get_{p.Name}", GetterParams);
            w.WriteLine($"return self[{bkey}]");
            EmitMethodClose(w);
            EmitMethodOpen(w, script, $"set_{p.Name}", SetterParams);
            w.WriteLine($"self[{bkey}] = value");
            EmitMethodClose(w);
        }
        else
        {
            if (readable && p.ReadHandler != null)
                EmitFunction(w, script, p.ReadHandler, obj, forcedName: $"get_{p.Name}");
            if (writable && p.WriteHandler != null)
                EmitFunction(w, script, p.WriteHandler, obj, forcedName: $"set_{p.Name}");
        }
    }

    static void EmitState(LuaWriter w, string script, PexObject obj, PexState st)
    {
        w.WriteLine($"-- ---- State: {(st.Name == "" ? "(default)" : st.Name)} ----");

        string dispatch;
        if (st.Name == "")
        {
            dispatch = script;
        }
        else
        {
            string sanitized = SanitizeForLocal(st.Name);
            dispatch = $"__state_{sanitized}";
            w.WriteLine($"local {dispatch} = {{}}");
            w.WriteLine($"{script}.__states[{NameMangling.ToLuaStringLiteral(st.Name)}] = {dispatch}");
        }
        w.WriteLine();

        foreach (var f in st.Functions)
            EmitFunction(w, dispatch, f, obj);
    }

    static void EmitFunction(LuaWriter w, string target, PexFunction f, PexObject obj, string? forcedName = null)
    {
        if (f == null) return;
        string name = forcedName ?? f.Name;
        bool isGlobal = (f.Flags & 0x01) != 0;
        bool isNative = (f.Flags & 0x02) != 0;

        // Signature comment.
        var paramsDesc = string.Join(", ", f.Parameters.Select(p => $"{p.TypeName} {p.Name}"));
        var tags = new List<string>();
        if (isGlobal) tags.Add("global");
        if (isNative) tags.Add("native");
        string tagSuffix = tags.Count == 0 ? "" : $"  -- [{string.Join(", ", tags)}]";
        w.WriteLine($"-- {f.ReturnType} {name}({paramsDesc}){tagSuffix}");

        var argNames = new List<string>();
        if (!isGlobal) argNames.Add("self");
        argNames.AddRange(f.Parameters.Select(p => NameMangling.ToLocalIdentifier(p.Name)));

        EmitMethodOpen(w, target, name, argNames);

        if (isNative)
        {
            string msg = NameMangling.ToLuaStringLiteral($"NATIVE_UNIMPLEMENTED: {target}.{name}");
            w.WriteLine($"error({msg}, 2)");
        }
        else
        {
            foreach (var loc in f.Locals)
                w.WriteLine($"local {NameMangling.ToLocalIdentifier(loc.Name)}  -- {loc.TypeName}");
            var ctx = new FunctionContext(obj, f);
            OpcodeLowering.LowerFunction(w, ctx, f.Instructions);
        }

        EmitMethodClose(w);
    }

    static void EmitMethodOpen(LuaWriter w, string target, string name, IEnumerable<string> paramNames)
    {
        string argList = string.Join(", ", paramNames);
        if (NameMangling.IsValidLuaIdentifier(name))
        {
            w.WriteLine($"function {target}.{name}({argList})");
        }
        else
        {
            // Fall back to bracket-indexed assignment — legal for any string key.
            string key = NameMangling.ToLuaStringLiteral(name);
            w.WriteLine($"{target}[{key}] = function({argList})");
        }
        w.Indent();
    }

    static void EmitMethodClose(LuaWriter w)
    {
        w.Dedent();
        w.WriteLine("end");
        w.WriteLine();
    }

    static string SanitizeForLocal(string name)
    {
        var sb = new StringBuilder(name.Length);
        foreach (char c in name)
            sb.Append(char.IsAsciiLetterOrDigit(c) ? c : '_');
        string s = sb.Length == 0 ? "_" : sb.ToString();
        if (char.IsDigit(s[0])) s = "_" + s;
        return s;
    }

    static string LiteralOf(PexValue v) => v switch
    {
        PexValue.None => "nil",
        PexValue.Bool b => b.Value ? "true" : "false",
        PexValue.IntValue i => i.Value.ToString(CultureInfo.InvariantCulture),
        PexValue.FloatValue f => LuaNumberFormat.FloatLiteral(f.Value),
        PexValue.Str s => NameMangling.ToLuaStringLiteral(s.Value),
        PexValue.Identifier id => $"nil  --[[ ident:{id.Name} ]]",
        _ => "nil",
    };
}
