// Immutable data model for a parsed Papyrus .pex file.
// Layout follows the v3.9 (Fallout 4) format documented at:
//   https://en.uesp.net/wiki/Skyrim_Mod:Compiled_Script_File_Format
// with FO4-specific additions (structs, property groups, struct orders,
// extra opcodes) confirmed against Champollion and Caprica.

using System.Text.Json.Serialization;

namespace AssetConverter.PapyrusLua.Pex;

public sealed record PexFile(
    PexHeader Header,
    string[] StringTable,
    PexDebugInfo? DebugInfo,       // null if file has no debug info
    PexUserFlag[] UserFlags,
    PexObject[] Objects);

public sealed record PexHeader(
    uint Magic,                    // 0xFA57C0DE
    byte MajorVersion,             // 3 for FO4/SkyrimSE
    byte MinorVersion,             // 9 for FO4, 2/3 for Skyrim
    ushort GameId,                 // 2 for FO4, 1 for SkyrimSE, 0 for SkyrimLE
    ulong CompileTime,             // Unix epoch
    string SourceFileName,
    string User,
    string Machine);

public sealed record PexUserFlag(string Name, byte BitIndex);

public sealed record PexDebugInfo(
    ulong ModificationTime,
    PexDebugFunction[] Functions,
    PexPropertyGroup[] PropertyGroups,   // FO4+
    PexStructOrder[] StructOrders);      // FO4+

public sealed record PexDebugFunction(
    string ObjectName,
    string StateName,
    string FunctionName,
    byte FunctionType,                   // 0=method, 1=propget, 2=propset, 3=event
    ushort[] LineNumbers);

public sealed record PexPropertyGroup(
    string ObjectName,
    string GroupName,
    string DocString,
    uint UserFlags,
    string[] PropertyNames);

public sealed record PexStructOrder(
    string ObjectName,
    string OrderName,
    string[] MemberNames);

public sealed record PexObject(
    string Name,
    uint Size,                           // byte size of the remainder of this object; diagnostic only
    string ParentClassName,
    string DocString,
    byte ConstFlag,                      // FO4+: non-zero if the object is declared Const
    uint UserFlags,
    string AutoStateName,
    PexStruct[] Structs,                 // FO4+
    PexVariable[] Variables,
    PexProperty[] Properties,
    PexState[] States);

public sealed record PexStruct(
    string Name,
    PexStructMember[] Members);

public sealed record PexStructMember(
    string Name,
    string TypeName,
    uint UserFlags,
    PexValue InitialValue,
    byte ConstFlag,
    string DocString);

public sealed record PexVariable(
    string Name,
    string TypeName,
    uint UserFlags,
    PexValue InitialValue,
    byte ConstFlag);

public sealed record PexProperty(
    string Name,
    string TypeName,
    string DocString,
    uint UserFlags,
    byte Flags,                          // bit0 read, bit1 write, bit2 autoVar
    string AutoVarName,                  // "" when not auto-var
    PexFunction? ReadHandler,            // null unless readable && !autoVar
    PexFunction? WriteHandler);           // null unless writable && !autoVar

public sealed record PexState(
    string Name,                         // "" for the default (auto) state
    PexFunction[] Functions);

public sealed record PexFunction(
    string Name,
    string ReturnType,
    string DocString,
    uint UserFlags,
    byte Flags,                          // bit0 global, bit1 native
    PexParameter[] Parameters,
    PexLocal[] Locals,
    PexInstruction[] Instructions);

public sealed record PexParameter(string Name, string TypeName);
public sealed record PexLocal(string Name, string TypeName);

public sealed record PexInstruction(
    OpCode OpCode,
    PexValue[] Operands);                // variadic opcodes include all args here

// Tagged union of the six PEX value types. Operand reads dispatch on the leading type byte.
[JsonPolymorphic(TypeDiscriminatorPropertyName = "kind")]
[JsonDerivedType(typeof(PexValue.None), "none")]
[JsonDerivedType(typeof(PexValue.Identifier), "ident")]
[JsonDerivedType(typeof(PexValue.Str), "str")]
[JsonDerivedType(typeof(PexValue.IntValue), "int")]
[JsonDerivedType(typeof(PexValue.FloatValue), "float")]
[JsonDerivedType(typeof(PexValue.Bool), "bool")]
public abstract record PexValue
{
    public sealed record None : PexValue
    {
        [JsonIgnore] public static readonly None Instance = new();
        public override string ToString() => "None";
    }
    public sealed record Identifier(string Name) : PexValue;
    public sealed record Str(string Value) : PexValue;
    public sealed record IntValue(int Value) : PexValue;
    public sealed record FloatValue(float Value) : PexValue;
    public sealed record Bool(bool Value) : PexValue;
}
