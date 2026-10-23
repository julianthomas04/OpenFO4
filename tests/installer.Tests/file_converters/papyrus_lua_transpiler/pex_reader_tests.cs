using System.Text;
using AssetConverter.PapyrusLua.Pex;
using InstallerTests.Support;
using Xunit;
using static InstallerTests.Support.PexModel;

namespace InstallerTests
{
    // Layout details these tests pin (an earlier revision of the page had them wrong):
    //   - header strings (source/user/machine) are inline u16-length strings, not string-table references;
    //   - PexValue tags are 1 identifier, 2 string, 3 int, 4 float, 5 bool;
    //   - a property stores userFlags (u32) before flags (u8).
    public class PexReaderTests
    {
        [Fact]
        public void Read_HandWrittenMinimalFile_DecodesTheDocumentedHeader()
        {
            byte[] bytes =
            [
                0xDE, 0xC0, 0x57, 0xFA,                         // magic 0xFA57C0DE little endian
                0x03, 0x09,                                     // version 3.9
                0x02, 0x00,                                     // game id 2 = Fallout 4
                0x01, 0x02, 0x03, 0x04, 0x00, 0x00, 0x00, 0x00, // compile time
                0x02, 0x00, (byte)'A', (byte)'b',               // source "Ab"
                0x01, 0x00, (byte)'u',                          // user "u"
                0x01, 0x00, (byte)'m',                          // machine "m"
                0x00, 0x00,                                     // string table: 0 entries
                0x00,                                           // no debug info
                0x00, 0x00,                                     // 0 user flags
                0x00, 0x00,                                     // 0 objects
            ];

            var pex = Read(bytes);

            Assert.Equal(0xFA57C0DEu, pex.Header.Magic);
            Assert.Equal(3, pex.Header.MajorVersion);
            Assert.Equal(9, pex.Header.MinorVersion);
            Assert.Equal(2, pex.Header.GameId);
            Assert.Equal(0x04030201UL, pex.Header.CompileTime);
            Assert.Equal("Ab", pex.Header.SourceFileName);
            Assert.Equal("u", pex.Header.User);
            Assert.Equal("m", pex.Header.Machine);
            Assert.Empty(pex.StringTable);
            Assert.Null(pex.DebugInfo);
            Assert.Empty(pex.UserFlags);
            Assert.Empty(pex.Objects);
        }

        [Fact]
        public void Read_BadMagic_ThrowsNamingTheMagic()
        {
            byte[] bytes = PexBytes.Write(File());
            bytes[0] = 0x00;

            var ex = Assert.Throws<InvalidDataException>(() => Read(bytes));

            Assert.Contains("bad magic", ex.Message);
        }

        [Theory]
        [InlineData(0)]
        [InlineData(1)]
        public void Read_SkyrimGameId_IsRejected(ushort gameId)
        {
            var pex = File();
            pex = pex with { Header = pex.Header with { GameId = gameId } };

            var ex = Assert.Throws<InvalidDataException>(() => Read(PexBytes.Write(pex)));

            Assert.Contains("unsupported gameId", ex.Message);
        }

        [Fact]
        public void Read_NonAsciiInlineString_IsDecodedAsUtf8()
        {
            var pex = File();
            pex = pex with { Header = pex.Header with { SourceFileName = "Skript_äö.psc" } };

            var read = Read(PexBytes.Write(pex));

            Assert.Equal("Skript_äö.psc", read.Header.SourceFileName);
        }

        [Fact]
        public void Read_HeaderStringInWindows1252_IsDecodedAsThatCodePage()
        {
            var pex = File();
            pex = pex with { Header = pex.Header with { SourceFileName = "Skript_ä€.psc" } };
            byte[] utf8Form = PexBytes.Write(pex);
            byte[] utf8Name = System.Text.Encoding.UTF8.GetBytes("Skript_ä€.psc");
            byte[] ansiName = [.. "Skript_"u8, 0xE4, 0x80, .. ".psc"u8];
            int nameAt = utf8Form.AsSpan().IndexOf(utf8Name);

            byte[] ansiForm = [.. utf8Form[..(nameAt - 2)], (byte)ansiName.Length, 0, .. ansiName, .. utf8Form[(nameAt + utf8Name.Length)..]];

            Assert.Equal("Skript_ä€.psc", Read(ansiForm).Header.SourceFileName);
        }

        [Fact]
        public void Read_FullObjectGraph_RoundTripsEveryLevel()
        {
            var structs = new[]
            {
                new PexStruct("Pair", [new PexStructMember("Left", "Int", 1, Int(-5), 1, "doc left"), new PexStructMember("Right", "String", 2, Str("r"), 0, "")]),
            };
            var variables = new[]
            {
                new PexVariable("count", "Int", 0, Int(int.MinValue), 0),
                new PexVariable("ratio", "Float", 0, Float(0.5f), 1),
                new PexVariable("flag", "Bool", 0, Bool(true), 0),
                new PexVariable("ref", "Form", 0, None, 0),
                new PexVariable("alias", "Form", 0, Id("other"), 0),
            };
            var getter = Function("", [Op(OpCode.Return, Int(1))], returnType: "Int");
            var setter = Function("", [Op(OpCode.Return, None)], parameters: [("value", "Int")]);
            var properties = new[]
            {
                new PexProperty("Auto", "Int", "d", 7, 0x01 | 0x02 | 0x04, "::Auto_var", null, null),
                new PexProperty("ReadOnly", "Int", "", 0, 0x01, "", getter, null),
                new PexProperty("ReadWrite", "Int", "", 0, 0x03, "", getter, setter),
                new PexProperty("WriteOnly", "Int", "", 0, 0x02, "", null, setter),
            };
            var states = new[]
            {
                new PexState("", [Function("Main", [Op(OpCode.Nop)], [("a", "Int")], [("t", "Float")], "Bool", flags: 0x01)]),
                new PexState("Busy", [Function("Wait", [Op(OpCode.Return, None)], flags: 0x02)]),
            };
            var obj = new PexObject("MyScript", 0, "Parent", "doc", 1, 3, "Busy", structs, variables, properties, states);

            var read = Read(PexBytes.Write(File(obj))).Objects.Single();

            Assert.Equal("MyScript", read.Name);
            Assert.Equal("Parent", read.ParentClassName);
            Assert.Equal("doc", read.DocString);
            Assert.Equal(1, read.ConstFlag);
            Assert.Equal(3u, read.UserFlags);
            Assert.Equal("Busy", read.AutoStateName);
            Assert.True(read.Size > 0);

            var pair = read.Structs.Single();
            Assert.Equal("Pair", pair.Name);
            Assert.Equal(new PexStructMember("Left", "Int", 1, Int(-5), 1, "doc left"), pair.Members[0]);
            Assert.Equal(new PexStructMember("Right", "String", 2, Str("r"), 0, ""), pair.Members[1]);

            Assert.Equal(new[] { Int(int.MinValue), Float(0.5f), Bool(true), None, Id("other") }, read.Variables.Select(v => v.InitialValue));
            Assert.Equal(1, read.Variables[1].ConstFlag);

            Assert.Equal("::Auto_var", read.Properties[0].AutoVarName);
            Assert.Null(read.Properties[0].ReadHandler);
            Assert.Null(read.Properties[0].WriteHandler);
            Assert.NotNull(read.Properties[1].ReadHandler);
            Assert.Null(read.Properties[1].WriteHandler);
            Assert.NotNull(read.Properties[2].ReadHandler);
            Assert.NotNull(read.Properties[2].WriteHandler);
            Assert.Null(read.Properties[3].ReadHandler);
            Assert.Equal("value", read.Properties[3].WriteHandler!.Parameters.Single().Name);
            Assert.Equal(7u, read.Properties[0].UserFlags);

            Assert.Equal(new[] { "", "Busy" }, read.States.Select(s => s.Name));
            var main = read.States[0].Functions.Single();
            Assert.Equal("Main", main.Name);
            Assert.Equal("Bool", main.ReturnType);
            Assert.Equal(1, main.Flags);
            Assert.Equal(new PexParameter("a", "Int"), main.Parameters.Single());
            Assert.Equal(new PexLocal("t", "Float"), main.Locals.Single());
            Assert.Equal(OpCode.Nop, main.Instructions.Single().OpCode);
            Assert.Equal(2, read.States[1].Functions.Single().Flags);
        }

        [Fact]
        public void Read_DebugInfoAndUserFlags_AreDecoded()
        {
            var debug = new PexDebugInfo(
                1234UL,
                [new PexDebugFunction("S", "", "F", 3, [10, 20, 65535])],
                [new PexPropertyGroup("S", "Grp", "docs", 5, ["P1", "P2"])],
                [new PexStructOrder("S", "Ord", ["m1", "m2"])]);
            var pex = File() with { DebugInfo = debug, UserFlags = [new PexUserFlag("Hidden", 0), new PexUserFlag("Conditional", 3)] };

            var read = Read(PexBytes.Write(pex));

            Assert.Equal(1234UL, read.DebugInfo!.ModificationTime);
            var function = read.DebugInfo.Functions.Single();
            Assert.Equal(("S", "", "F", (byte)3), (function.ObjectName, function.StateName, function.FunctionName, function.FunctionType));
            Assert.Equal(new ushort[] { 10, 20, 65535 }, function.LineNumbers);
            var group = read.DebugInfo.PropertyGroups.Single();
            Assert.Equal(("Grp", "docs", 5u), (group.GroupName, group.DocString, group.UserFlags));
            Assert.Equal(new[] { "P1", "P2" }, group.PropertyNames);
            Assert.Equal(new[] { "m1", "m2" }, read.DebugInfo.StructOrders.Single().MemberNames);
            Assert.Equal(new[] { ("Hidden", (byte)0), ("Conditional", (byte)3) }, read.UserFlags.Select(f => (f.Name, f.BitIndex)));
        }

        [Fact]
        public void Read_FixedArityInstruction_KeepsEachOperandWithItsType()
        {
            var function = Function("F", [Op(OpCode.IAdd, Id("dest"), Int(7), Float(1.5f))]);

            var read = ReadSingleFunction(function);

            Assert.Equal(new[] { Id("dest"), Int(7), Float(1.5f) }, read.Instructions.Single().Operands);
        }

        [Fact]
        public void Read_VariadicCall_FlattensFixedOperandsCountAndArguments()
        {
            var call = Op(OpCode.CallMethod, Str("Foo"), Id("self"), Id("::NoneVar"), Int(2), Bool(true), Str("x"));

            var read = ReadSingleFunction(Function("F", [call]));

            Assert.Equal(call.Operands, read.Instructions.Single().Operands);
        }

        [Fact]
        public void Read_VariadicCallWithZeroArguments_ReadsNoExtraOperands()
        {
            var call = Op(OpCode.CallStatic, Str("Game"), Str("GetPlayer"), Id("p"), Int(0));

            var read = ReadSingleFunction(Function("F", [call, Op(OpCode.Return, Id("p"))]));

            Assert.Equal(2, read.Instructions.Length);
            Assert.Equal(4, read.Instructions[0].Operands.Length);
        }

        [Fact]
        public void Read_LockGuardsWithNoFixedOperands_StartsWithTheArgumentCount()
        {
            var lockGuards = Op(OpCode.LockGuards, Int(2), Id("g1"), Id("g2"));

            var read = ReadSingleFunction(Function("F", [lockGuards]));

            Assert.Equal(lockGuards.Operands, read.Instructions.Single().Operands);
        }

        [Theory]
        [InlineData(-1)]
        [InlineData(1025)]
        public void Read_ImplausibleVariadicArgumentCount_ThrowsInvalidData(int count)
        {
            var function = Function("F", [Op(OpCode.CallParent, Str("M"), Id("d"), Int(count))]);

            var ex = Assert.Throws<InvalidDataException>(() => Read(PexBytes.Write(File(Script("S", function)))));

            Assert.Contains("implausible arg count", ex.Message);
        }

        [Fact]
        public void Read_VariadicArgumentCountOfWrongType_ThrowsInvalidData()
        {
            var function = Function("F", [Op(OpCode.CallParent, Str("M"), Id("d"), Str("two"))]);

            var ex = Assert.Throws<InvalidDataException>(() => Read(PexBytes.Write(File(Script("S", function)))));

            Assert.Contains("expected int arg count", ex.Message);
        }

        [Fact]
        public void Read_UnassignedOpcodeByte_ThrowsInvalidData()
        {
            byte[] bytes = PexBytes.Write(File(Script("S", Function("F", [Op(OpCode.Nop)]))));
            int nopOffset = Array.LastIndexOf(bytes, (byte)OpCode.Nop);
            bytes[nopOffset] = 0xC8;

            var ex = Assert.Throws<InvalidDataException>(() => Read(bytes));

            Assert.Contains("Unhandled opcode: 0xC8", ex.Message);
        }

        [Fact]
        public void Read_UnknownOperandTypeTag_ThrowsInvalidData()
        {
            byte[] bytes = PexBytes.Write(File(Script("S", Function("F", [Op(OpCode.Return, None)]))));
            bytes[^1] = 9;

            var ex = Assert.Throws<InvalidDataException>(() => Read(bytes));

            Assert.Contains("Unknown operand type byte: 0x09", ex.Message);
        }

        [Fact]
        public void Read_StringReferencePastTheTable_ThrowsInvalidData()
        {
            byte[] bytes = PexBytes.Write(File(Script("S", Function("F", [Op(OpCode.Return, Id("x"))]))));
            // The operand's string index is the last two bytes of the file.
            bytes[^2] = 0xFF;
            bytes[^1] = 0x7F;

            var ex = Assert.Throws<InvalidDataException>(() => Read(bytes));

            Assert.Contains("String ref 32767 out of range", ex.Message);
        }

        [Fact]
        public void Read_StringLongerThanTheRemainingBytes_ThrowsEndOfStream()
        {
            var bytes = new List<byte> { 0xDE, 0xC0, 0x57, 0xFA, 3, 9, 2, 0, 0, 0, 0, 0, 0, 0, 0, 0 };
            bytes.AddRange(BitConverter.GetBytes((ushort)500));
            bytes.AddRange(Encoding.UTF8.GetBytes("short"));

            Assert.Throws<EndOfStreamException>(() => Read(bytes.ToArray()));
        }

        [Fact]
        public void Read_EveryTruncationOfAValidFile_ThrowsEndOfStream()
        {
            byte[] valid = PexBytes.Write(File(new PexObject(
                "S", 0, "P", "", 0, 0, "", [], [new PexVariable("v", "Int", 0, Int(1), 0)],
                [new PexProperty("A", "Int", "", 0, 0x05, "::A", null, null)],
                [new PexState("", [Function("F", [Op(OpCode.CallMethod, Str("m"), Id("self"), Id("r"), Int(1), Int(2))])])])));
            var truncations = Enumerable.Range(0, valid.Length).Select(length => valid[..length]).ToList();

            Assert.All(truncations, truncated => Assert.Throws<EndOfStreamException>(() => Read(truncated)));
        }

        // Robustness: arbitrary bytes behind a valid header may only produce the two documented exceptions.
        [Fact]
        public void Read_RandomBytesBehindAValidHeader_OnlyThrowsInvalidDataOrEndOfStream()
        {
            byte[] header = PexBytes.Write(File())[..16];
            var random = new Random(20241012);
            var buffers = Enumerable.Range(0, 400).Select(_ =>
            {
                byte[] tail = new byte[random.Next(0, 300)];
                random.NextBytes(tail);
                return header.Concat(tail).ToArray();
            }).ToList();

            Assert.All(buffers, buffer =>
            {
                var ex = Record.Exception(() => Read(buffer));
                Assert.True(ex is null or InvalidDataException or EndOfStreamException, $"unexpected {ex?.GetType().Name}: {ex?.Message}");
            });
        }

        private static PexFile Read(byte[] bytes)
        {
            using var reader = new BinaryReader(new MemoryStream(bytes));
            return PexReader.Read(reader, "test.pex");
        }

        private static PexFunction ReadSingleFunction(PexFunction function) =>
            Read(PexBytes.Write(File(Script("S", function)))).Objects.Single().States.Single().Functions.Single();
    }
}
