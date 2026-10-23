using AssetConverter.PapyrusLua.Pex;

namespace InstallerTests.Support
{
    /// <summary>Short constructors for hand-built PEX models.</summary>
    internal static class PexModel
    {
        public static PexValue Id(string name) => new PexValue.Identifier(name);
        public static PexValue Str(string value) => new PexValue.Str(value);
        public static PexValue Int(int value) => new PexValue.IntValue(value);
        public static PexValue Float(float value) => new PexValue.FloatValue(value);
        public static PexValue Bool(bool value) => new PexValue.Bool(value);
        public static PexValue None => PexValue.None.Instance;

        public static PexInstruction Op(OpCode opCode, params PexValue[] operands) => new(opCode, operands);

        public static PexFunction Function(
            string name,
            IEnumerable<PexInstruction> instructions,
            (string Name, string Type)[]? parameters = null,
            (string Name, string Type)[]? locals = null,
            string returnType = "None",
            byte flags = 0) => new(
                name, returnType, "", 0, flags,
                (parameters ?? []).Select(p => new PexParameter(p.Name, p.Type)).ToArray(),
                (locals ?? []).Select(l => new PexLocal(l.Name, l.Type)).ToArray(),
                instructions.ToArray());

        public static PexObject Script(string name, params PexFunction[] defaultStateFunctions) =>
            Script(name, [new PexState("", defaultStateFunctions)]);

        public static PexObject Script(string name, PexState[] states, PexVariable[]? variables = null, PexProperty[]? properties = null, PexStruct[]? structs = null) => new(
            name, 0, "", "", 0, 0, "", structs ?? [], variables ?? [], properties ?? [], states);

        public static PexFile File(params PexObject[] objects) =>
            new(PexBytes.Fo4Header(), [], null, [], objects);
    }
}
