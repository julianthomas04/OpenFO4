namespace AssetConverter.Abc
{
    internal enum AbcOperand
    {
        U8,
        S8,
        U30,
        S24,

        /// <summary>A variable-length 32-bit value the compiler may sign-extend past 30 bits (pushshort).</summary>
        U32,
    }

    internal sealed record AbcOpcodeInfo(string Mnemonic, AbcOperand[] Operands);

    /// <summary>The AVM2 instruction set: mnemonic and operand encoding of every opcode. <c>lookupswitch</c> and <c>debug</c> have their own layouts.</summary>
    internal static class AbcOpcodes
    {
        public const byte IfNlt = 0x0C;
        public const byte IfNle = 0x0D;
        public const byte IfNgt = 0x0E;
        public const byte IfNge = 0x0F;
        public const byte Jump = 0x10;
        public const byte IfTrue = 0x11;
        public const byte IfFalse = 0x12;
        public const byte IfEq = 0x13;
        public const byte IfNe = 0x14;
        public const byte IfLt = 0x15;
        public const byte IfLe = 0x16;
        public const byte IfGt = 0x17;
        public const byte IfGe = 0x18;
        public const byte IfStrictEq = 0x19;
        public const byte IfStrictNe = 0x1A;
        public const byte LookupSwitch = 0x1B;
        public const byte Throw = 0x03;
        public const byte ReturnVoid = 0x47;
        public const byte ReturnValue = 0x48;
        public const byte Debug = 0xEF;

        private static readonly AbcOpcodeInfo?[] Table = Build();

        public static AbcOpcodeInfo Info(byte opcode) =>
            Table[opcode] ?? throw new AbcFormatException($"0x{opcode:x2} is not an AVM2 opcode.");

        public static bool IsBranch(byte opcode) => opcode is >= IfNlt and <= IfStrictNe;

        public static bool IsConditionalBranch(byte opcode) => IsBranch(opcode) && opcode != Jump;

        public static bool EndsFlow(byte opcode) => opcode is Jump or Throw or ReturnVoid or ReturnValue or LookupSwitch;

        private static AbcOpcodeInfo?[] Build()
        {
            var table = new AbcOpcodeInfo?[256];
            void Op(int code, string mnemonic, params AbcOperand[] operands) => table[code] = new AbcOpcodeInfo(mnemonic, operands);
            const AbcOperand U30 = AbcOperand.U30;

            Op(0x01, "bkpt");
            Op(0x02, "nop");
            Op(0x03, "throw");
            Op(0x04, "getsuper", U30);
            Op(0x05, "setsuper", U30);
            Op(0x06, "dxns", U30);
            Op(0x07, "dxnslate");
            Op(0x08, "kill", U30);
            Op(0x09, "label");
            string[] branches = ["ifnlt", "ifnle", "ifngt", "ifnge", "jump", "iftrue", "iffalse", "ifeq", "ifne", "iflt", "ifle", "ifgt", "ifge", "ifstricteq", "ifstrictne"];
            for (int i = 0; i < branches.Length; i++)
                Op(IfNlt + i, branches[i], AbcOperand.S24);
            Op(0x1B, "lookupswitch");
            Op(0x1C, "pushwith");
            Op(0x1D, "popscope");
            Op(0x1E, "nextname");
            Op(0x1F, "hasnext");
            Op(0x20, "pushnull");
            Op(0x21, "pushundefined");
            Op(0x23, "nextvalue");
            Op(0x24, "pushbyte", AbcOperand.S8);
            Op(0x25, "pushshort", AbcOperand.U32);
            Op(0x26, "pushtrue");
            Op(0x27, "pushfalse");
            Op(0x28, "pushnan");
            Op(0x29, "pop");
            Op(0x2A, "dup");
            Op(0x2B, "swap");
            Op(0x2C, "pushstring", U30);
            Op(0x2D, "pushint", U30);
            Op(0x2E, "pushuint", U30);
            Op(0x2F, "pushdouble", U30);
            Op(0x30, "pushscope");
            Op(0x31, "pushnamespace", U30);
            Op(0x32, "hasnext2", U30, U30);
            string[] memory = ["li8", "li16", "li32", "lf32", "lf64", "si8", "si16", "si32", "sf32", "sf64"];
            for (int i = 0; i < memory.Length; i++)
                Op(0x35 + i, memory[i]);
            Op(0x40, "newfunction", U30);
            Op(0x41, "call", U30);
            Op(0x42, "construct", U30);
            Op(0x43, "callmethod", U30, U30);
            Op(0x44, "callstatic", U30, U30);
            Op(0x45, "callsuper", U30, U30);
            Op(0x46, "callproperty", U30, U30);
            Op(0x47, "returnvoid");
            Op(0x48, "returnvalue");
            Op(0x49, "constructsuper", U30);
            Op(0x4A, "constructprop", U30, U30);
            Op(0x4C, "callproplex", U30, U30);
            Op(0x4E, "callsupervoid", U30, U30);
            Op(0x4F, "callpropvoid", U30, U30);
            Op(0x50, "sxi1");
            Op(0x51, "sxi8");
            Op(0x52, "sxi16");
            Op(0x53, "applytype", U30);
            Op(0x55, "newobject", U30);
            Op(0x56, "newarray", U30);
            Op(0x57, "newactivation");
            Op(0x58, "newclass", U30);
            Op(0x59, "getdescendants", U30);
            Op(0x5A, "newcatch", U30);
            Op(0x5D, "findpropstrict", U30);
            Op(0x5E, "findproperty", U30);
            Op(0x5F, "finddef", U30);
            Op(0x60, "getlex", U30);
            Op(0x61, "setproperty", U30);
            Op(0x62, "getlocal", U30);
            Op(0x63, "setlocal", U30);
            Op(0x64, "getglobalscope");
            Op(0x65, "getscopeobject", AbcOperand.U8);
            Op(0x66, "getproperty", U30);
            Op(0x67, "getouterscope", U30);
            Op(0x68, "initproperty", U30);
            Op(0x6A, "deleteproperty", U30);
            Op(0x6C, "getslot", U30);
            Op(0x6D, "setslot", U30);
            Op(0x6E, "getglobalslot", U30);
            Op(0x6F, "setglobalslot", U30);
            Op(0x70, "convert_s");
            Op(0x71, "esc_xelem");
            Op(0x72, "esc_xattr");
            Op(0x73, "convert_i");
            Op(0x74, "convert_u");
            Op(0x75, "convert_d");
            Op(0x76, "convert_b");
            Op(0x77, "convert_o");
            Op(0x78, "checkfilter");
            Op(0x80, "coerce", U30);
            Op(0x81, "coerce_b");
            Op(0x82, "coerce_a");
            Op(0x83, "coerce_i");
            Op(0x84, "coerce_d");
            Op(0x85, "coerce_s");
            Op(0x86, "astype", U30);
            Op(0x87, "astypelate");
            Op(0x88, "coerce_u");
            Op(0x89, "coerce_o");
            Op(0x90, "negate");
            Op(0x91, "increment");
            Op(0x92, "inclocal", U30);
            Op(0x93, "decrement");
            Op(0x94, "declocal", U30);
            Op(0x95, "typeof");
            Op(0x96, "not");
            Op(0x97, "bitnot");
            string[] binary = ["add", "subtract", "multiply", "divide", "modulo", "lshift", "rshift", "urshift", "bitand", "bitor", "bitxor",
                "equals", "strictequals", "lessthan", "lessequals", "greaterthan", "greaterequals", "instanceof"];
            for (int i = 0; i < binary.Length; i++)
                Op(0xA0 + i, binary[i]);
            Op(0xB2, "istype", U30);
            Op(0xB3, "istypelate");
            Op(0xB4, "in");
            Op(0xC0, "increment_i");
            Op(0xC1, "decrement_i");
            Op(0xC2, "inclocal_i", U30);
            Op(0xC3, "declocal_i", U30);
            Op(0xC4, "negate_i");
            Op(0xC5, "add_i");
            Op(0xC6, "subtract_i");
            Op(0xC7, "multiply_i");
            for (int i = 0; i < 4; i++)
            {
                Op(0xD0 + i, $"getlocal{i}");
                Op(0xD4 + i, $"setlocal{i}");
            }
            Op(0xEF, "debug");
            Op(0xF0, "debugline", U30);
            Op(0xF1, "debugfile", U30);
            Op(0xF2, "bkptline", U30);
            Op(0xF3, "timestamp");
            return table;
        }
    }

    /// <summary>One decoded instruction: its byte offset, opcode, operands in encoding order, and for lookupswitch its targets (default first).</summary>
    internal sealed record AbcInstruction(int Offset, int Length, byte Opcode, IReadOnlyList<int> Operands, IReadOnlyList<int> SwitchTargets)
    {
        public string Mnemonic => AbcOpcodes.Info(Opcode).Mnemonic;

        public int Next => Offset + Length;

        /// <summary>The absolute target of a branch (offsets are relative to the next instruction).</summary>
        public int BranchTarget => Next + Operands[0];
    }

    internal static class AbcDecoder
    {
        public static IReadOnlyList<AbcInstruction> Decode(byte[] code, string where)
        {
            var instructions = new List<AbcInstruction>();
            var bytes = new AbcBytes(code, 0, code.Length);
            try
            {
                while (!bytes.AtEnd)
                {
                    int offset = bytes.Position;
                    byte opcode = bytes.U8();
                    AbcOpcodeInfo info = AbcOpcodes.Info(opcode);
                    var operands = new List<int>(info.Operands.Length);
                    IReadOnlyList<int> switchTargets = [];
                    if (opcode == AbcOpcodes.LookupSwitch)
                    {
                        int defaultTarget = bytes.S24();
                        int caseCount = bytes.U30();
                        var targets = new List<int>(caseCount + 2) { offset + defaultTarget };
                        for (int i = 0; i <= caseCount; i++)
                            targets.Add(offset + bytes.S24());
                        switchTargets = targets;
                    }
                    else if (opcode == AbcOpcodes.Debug)
                    {
                        operands.Add(bytes.U8());
                        operands.Add(bytes.U30());
                        operands.Add(bytes.U8());
                        operands.Add(bytes.U30());
                    }
                    else
                    {
                        foreach (AbcOperand operand in info.Operands)
                            operands.Add(operand switch
                            {
                                AbcOperand.U8 => bytes.U8(),
                                AbcOperand.S8 => bytes.S8(),
                                AbcOperand.S24 => bytes.S24(),
                                AbcOperand.U32 => unchecked((int)bytes.U32()),
                                _ => bytes.U30(),
                            });
                    }
                    instructions.Add(new AbcInstruction(offset, bytes.Position - offset, opcode, operands, switchTargets));
                }
            }
            catch (AbcFormatException e)
            {
                throw new AbcFormatException($"{where}: {e.Message}");
            }
            return instructions;
        }
    }
}
