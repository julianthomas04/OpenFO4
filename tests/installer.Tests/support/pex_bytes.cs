using System.Text;
using AssetConverter.PapyrusLua.Pex;

namespace InstallerTests.Support
{
    /// <summary>Serialises a <see cref="PexFile"/> model to Fallout 4 PEX bytes (the PEX format plus the
    /// header/value-tag corrections listed in pex_reader_tests.cs). String references are interned into a string table.</summary>
    internal static class PexBytes
    {
        public static byte[] Write(PexFile pex)
        {
            var table = new List<string>();
            using var rest = new MemoryStream();
            using (var w = new BinaryWriter(rest, Encoding.UTF8, leaveOpen: true))
            {
                WriteDebugInfo(w, table, pex.DebugInfo);
                w.Write((ushort)pex.UserFlags.Length);
                foreach (var flag in pex.UserFlags)
                {
                    StrRef(w, table, flag.Name);
                    w.Write(flag.BitIndex);
                }
                w.Write((ushort)pex.Objects.Length);
                foreach (var obj in pex.Objects)
                    WriteObject(w, table, obj);
            }

            using var file = new MemoryStream();
            using (var w = new BinaryWriter(file, Encoding.UTF8, leaveOpen: true))
            {
                w.Write(pex.Header.Magic);
                w.Write(pex.Header.MajorVersion);
                w.Write(pex.Header.MinorVersion);
                w.Write(pex.Header.GameId);
                w.Write(pex.Header.CompileTime);
                InlineString(w, pex.Header.SourceFileName);
                InlineString(w, pex.Header.User);
                InlineString(w, pex.Header.Machine);
                w.Write((ushort)table.Count);
                foreach (string s in table)
                    InlineString(w, s);
                w.Write(rest.ToArray());
            }
            return file.ToArray();
        }

        public static PexHeader Fo4Header(string source = "Test.psc") =>
            new(0xFA57C0DEu, 3, 9, 2, 1_700_000_000UL, source, "user", "machine");

        private static void WriteDebugInfo(BinaryWriter w, List<string> table, PexDebugInfo? debug)
        {
            if (debug == null)
            {
                w.Write((byte)0);
                return;
            }
            w.Write((byte)1);
            w.Write(debug.ModificationTime);
            w.Write((ushort)debug.Functions.Length);
            foreach (var f in debug.Functions)
            {
                StrRef(w, table, f.ObjectName);
                StrRef(w, table, f.StateName);
                StrRef(w, table, f.FunctionName);
                w.Write(f.FunctionType);
                w.Write((ushort)f.LineNumbers.Length);
                foreach (ushort line in f.LineNumbers)
                    w.Write(line);
            }
            w.Write((ushort)debug.PropertyGroups.Length);
            foreach (var g in debug.PropertyGroups)
            {
                StrRef(w, table, g.ObjectName);
                StrRef(w, table, g.GroupName);
                StrRef(w, table, g.DocString);
                w.Write(g.UserFlags);
                w.Write((ushort)g.PropertyNames.Length);
                foreach (string name in g.PropertyNames)
                    StrRef(w, table, name);
            }
            w.Write((ushort)debug.StructOrders.Length);
            foreach (var o in debug.StructOrders)
            {
                StrRef(w, table, o.ObjectName);
                StrRef(w, table, o.OrderName);
                w.Write((ushort)o.MemberNames.Length);
                foreach (string name in o.MemberNames)
                    StrRef(w, table, name);
            }
        }

        private static void WriteObject(BinaryWriter w, List<string> table, PexObject obj)
        {
            StrRef(w, table, obj.Name);
            using var body = new MemoryStream();
            using (var b = new BinaryWriter(body, Encoding.UTF8, leaveOpen: true))
            {
                StrRef(b, table, obj.ParentClassName);
                StrRef(b, table, obj.DocString);
                b.Write(obj.ConstFlag);
                b.Write(obj.UserFlags);
                StrRef(b, table, obj.AutoStateName);

                b.Write((ushort)obj.Structs.Length);
                foreach (var s in obj.Structs)
                {
                    StrRef(b, table, s.Name);
                    b.Write((ushort)s.Members.Length);
                    foreach (var m in s.Members)
                    {
                        StrRef(b, table, m.Name);
                        StrRef(b, table, m.TypeName);
                        b.Write(m.UserFlags);
                        Value(b, table, m.InitialValue);
                        b.Write(m.ConstFlag);
                        StrRef(b, table, m.DocString);
                    }
                }

                b.Write((ushort)obj.Variables.Length);
                foreach (var v in obj.Variables)
                {
                    StrRef(b, table, v.Name);
                    StrRef(b, table, v.TypeName);
                    b.Write(v.UserFlags);
                    Value(b, table, v.InitialValue);
                    b.Write(v.ConstFlag);
                }

                b.Write((ushort)obj.Properties.Length);
                foreach (var p in obj.Properties)
                {
                    StrRef(b, table, p.Name);
                    StrRef(b, table, p.TypeName);
                    StrRef(b, table, p.DocString);
                    b.Write(p.UserFlags);
                    b.Write(p.Flags);
                    if ((p.Flags & 0x04) != 0)
                    {
                        StrRef(b, table, p.AutoVarName);
                        continue;
                    }
                    if ((p.Flags & 0x01) != 0)
                        Function(b, table, p.ReadHandler!, hasName: false);
                    if ((p.Flags & 0x02) != 0)
                        Function(b, table, p.WriteHandler!, hasName: false);
                }

                b.Write((ushort)obj.States.Length);
                foreach (var st in obj.States)
                {
                    StrRef(b, table, st.Name);
                    b.Write((ushort)st.Functions.Length);
                    foreach (var f in st.Functions)
                        Function(b, table, f, hasName: true);
                }
            }
            w.Write((uint)body.Length);
            w.Write(body.ToArray());
        }

        private static void Function(BinaryWriter w, List<string> table, PexFunction f, bool hasName)
        {
            if (hasName)
                StrRef(w, table, f.Name);
            StrRef(w, table, f.ReturnType);
            StrRef(w, table, f.DocString);
            w.Write(f.UserFlags);
            w.Write(f.Flags);
            w.Write((ushort)f.Parameters.Length);
            foreach (var p in f.Parameters)
            {
                StrRef(w, table, p.Name);
                StrRef(w, table, p.TypeName);
            }
            w.Write((ushort)f.Locals.Length);
            foreach (var l in f.Locals)
            {
                StrRef(w, table, l.Name);
                StrRef(w, table, l.TypeName);
            }
            w.Write((ushort)f.Instructions.Length);
            foreach (var instruction in f.Instructions)
            {
                w.Write((byte)instruction.OpCode);
                foreach (var operand in instruction.Operands)
                    Value(w, table, operand);
            }
        }

        /// <summary>Operand tag bytes as in the UESP compiled-script format: 0 none, 1 identifier, 2 string, 3 int, 4 float, 5 bool.</summary>
        public static void Value(BinaryWriter w, List<string> table, PexValue value)
        {
            switch (value)
            {
                case PexValue.None:
                    w.Write((byte)0);
                    break;
                case PexValue.Identifier id:
                    w.Write((byte)1);
                    StrRef(w, table, id.Name);
                    break;
                case PexValue.Str s:
                    w.Write((byte)2);
                    StrRef(w, table, s.Value);
                    break;
                case PexValue.IntValue i:
                    w.Write((byte)3);
                    w.Write(i.Value);
                    break;
                case PexValue.FloatValue f:
                    w.Write((byte)4);
                    w.Write(f.Value);
                    break;
                case PexValue.Bool b:
                    w.Write((byte)5);
                    w.Write((byte)(b.Value ? 1 : 0));
                    break;
                default:
                    throw new ArgumentException($"unsupported value {value}");
            }
        }

        public static void StrRef(BinaryWriter w, List<string> table, string value)
        {
            int index = table.IndexOf(value);
            if (index < 0)
            {
                index = table.Count;
                table.Add(value);
            }
            w.Write((ushort)index);
        }

        private static void InlineString(BinaryWriter w, string value)
        {
            byte[] bytes = Encoding.UTF8.GetBytes(value);
            w.Write((ushort)bytes.Length);
            w.Write(bytes);
        }
    }
}
