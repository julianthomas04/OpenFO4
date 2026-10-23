using AssetConverter.PapyrusLua.Lua;
using AssetConverter.PapyrusLua.Pex;
using InstallerTests.Support;
using MoonSharp.Interpreter;
using Xunit;
using static InstallerTests.Support.PexModel;

namespace InstallerTests
{
    // Functions are compiled to Lua by LuaEmitter and then executed
    // in MoonSharp, so expectations are the documented Papyrus results, not the generated text.
    public class OpcodeLoweringTests
    {
        private static readonly (string, string)[] AB = [("a", "Int"), ("b", "Int")];
        private static readonly (string, string)[] Result = [("result", "Int")];

        private static LuaRunner Binary(OpCode op) => LuaRunner.LoadFunction(Function(
            "Run", [Op(op, Id("result"), Id("a"), Id("b")), Op(OpCode.Return, Id("result"))], AB, Result, "Int", flags: 0x01));

        [Theory]
        [InlineData(OpCode.IAdd, 7, 5, 12)]
        [InlineData(OpCode.ISub, 7, 5, 2)]
        [InlineData(OpCode.IMul, 7, 5, 35)]
        [InlineData(OpCode.FAdd, 7, 5, 12)]
        [InlineData(OpCode.FSub, 7, 5, 2)]
        [InlineData(OpCode.FMul, 7, 5, 35)]
        public void Arithmetic_BinaryOperators_ProduceTheDocumentedResult(OpCode op, int a, int b, int expected)
        {
            Assert.Equal(expected, Binary(op).CallGlobal("Run", a, b).Number);
        }

        // IDiv truncates toward zero (IDiv / IMod); Lua's // would floor.
        [Theory]
        [InlineData(7, 2, 3)]
        [InlineData(-7, 2, -3)]
        [InlineData(7, -2, -3)]
        [InlineData(-7, -2, 3)]
        [InlineData(0, 5, 0)]
        public void IDiv_TruncatesTowardZero(int a, int b, int expected)
        {
            Assert.Equal(expected, Binary(OpCode.IDiv).CallGlobal("Run", a, b).Number);
        }

        // IMod follows the dividend's sign (C semantics): a - trunc(a/b)*b.
        [Theory]
        [InlineData(7, 3, 1)]
        [InlineData(-7, 3, -1)]
        [InlineData(7, -3, 1)]
        [InlineData(-7, -3, -1)]
        [InlineData(6, 3, 0)]
        public void IMod_SignFollowsTheDividend(int a, int b, int expected)
        {
            Assert.Equal(expected, Binary(OpCode.IMod).CallGlobal("Run", a, b).Number);
        }

        [Fact]
        public void FDiv_IsRealDivision()
        {
            Assert.Equal(0.25, Binary(OpCode.FDiv).CallGlobal("Run", 1, 4).Number);
        }

        [Theory]
        [InlineData(OpCode.INeg, 5, -5)]
        [InlineData(OpCode.INeg, -3, 3)]
        [InlineData(OpCode.FNeg, 0.5, -0.5)]
        public void Negation_FlipsTheSign(OpCode op, double input, double expected)
        {
            var runner = LuaRunner.LoadFunction(Function(
                "Run", [Op(op, Id("result"), Id("a")), Op(OpCode.Return, Id("result"))], [("a", "Int")], Result, "Int", flags: 0x01));

            Assert.Equal(expected, runner.CallGlobal("Run", input).Number);
        }

        [Theory]
        [InlineData(true, false)]
        [InlineData(false, true)]
        public void Not_InvertsTheBoolean(bool input, bool expected)
        {
            var runner = LuaRunner.LoadFunction(Function(
                "Run", [Op(OpCode.Not, Id("result"), Id("a")), Op(OpCode.Return, Id("result"))], [("a", "Bool")], [("result", "Bool")], "Bool", flags: 0x01));

            Assert.Equal(expected, runner.CallGlobal("Run", input).Boolean);
        }

        [Theory]
        [InlineData(OpCode.CmpEq, 2, 2, true)]
        [InlineData(OpCode.CmpEq, 2, 3, false)]
        [InlineData(OpCode.CmpLt, 2, 3, true)]
        [InlineData(OpCode.CmpLt, 3, 3, false)]
        [InlineData(OpCode.CmpLte, 3, 3, true)]
        [InlineData(OpCode.CmpLte, 4, 3, false)]
        [InlineData(OpCode.CmpGt, 4, 3, true)]
        [InlineData(OpCode.CmpGt, 3, 3, false)]
        [InlineData(OpCode.CmpGte, 3, 3, true)]
        [InlineData(OpCode.CmpGte, 2, 3, false)]
        public void Comparison_ReturnsTheBooleanOfTheRelation(OpCode op, int a, int b, bool expected)
        {
            var runner = LuaRunner.LoadFunction(Function(
                "Run", [Op(op, Id("result"), Id("a"), Id("b")), Op(OpCode.Return, Id("result"))], AB, [("result", "Bool")], "Bool", flags: 0x01));

            Assert.Equal(expected, runner.CallGlobal("Run", a, b).Boolean);
        }

        // Jump offsets are relative to the CURRENT instruction (jump offset convention).
        [Theory]
        [InlineData(0, 0)]
        [InlineData(1, 1)]
        [InlineData(4, 10)]
        [InlineData(10, 55)]
        public void Jumps_CountingLoop_SumsOneToN(int n, int expectedSum)
        {
            var runner = LuaRunner.LoadFunction(Function(
                "Sum",
                [
                    Op(OpCode.Assign, Id("sum"), Int(0)),
                    Op(OpCode.Assign, Id("i"), Int(1)),
                    Op(OpCode.CmpGt, Id("done"), Id("i"), Id("n")),     // 2
                    Op(OpCode.JmpT, Id("done"), Int(4)),                // 3 -> 7
                    Op(OpCode.IAdd, Id("sum"), Id("sum"), Id("i")),
                    Op(OpCode.IAdd, Id("i"), Id("i"), Int(1)),
                    Op(OpCode.Jmp, Int(-4)),                            // 6 -> 2
                    Op(OpCode.Return, Id("sum")),                       // 7
                ],
                [("n", "Int")], [("sum", "Int"), ("i", "Int"), ("done", "Bool")], "Int", flags: 0x01));

            Assert.Equal(expectedSum, runner.CallGlobal("Sum", n).Number);
        }

        [Theory]
        [InlineData(true, 11)]
        [InlineData(false, 22)]
        public void JmpF_SkipsTheFallThroughBranchWhenTheConditionIsFalse(bool condition, int expected)
        {
            var runner = LuaRunner.LoadFunction(Function(
                "Pick",
                [
                    Op(OpCode.JmpF, Id("c"), Int(3)),   // 0 -> 3
                    Op(OpCode.Assign, Id("r"), Int(11)),
                    Op(OpCode.Jmp, Int(2)),             // 2 -> 4
                    Op(OpCode.Assign, Id("r"), Int(22)),
                    Op(OpCode.Return, Id("r")),
                ],
                [("c", "Bool")], [("r", "Int")], "Int", flags: 0x01));

            Assert.Equal(expected, runner.CallGlobal("Pick", condition).Number);
        }

        [Theory]
        [InlineData(true)]
        [InlineData(false)]
        public void Jmp_TargetPastTheLastInstruction_ReturnsFromTheFunction(bool exitEarly)
        {
            var runner = LuaRunner.LoadFunction(Function(
                "Early",
                [
                    Op(OpCode.JmpT, Id("c"), Int(5)),           // 0 -> 5, past the end (2 instructions)
                    Op(OpCode.Return, Int(1)),
                ],
                [("c", "Bool")], returnType: "Int", flags: 0x01));

            var result = runner.CallGlobal("Early", exitEarly);

            Assert.Equal(exitEarly ? DataType.Void : DataType.Number, result.Type);
        }

        [Fact]
        public void Jmp_UnconditionalPastTheEnd_ReturnsWithoutValue()
        {
            var runner = LuaRunner.LoadFunction(Function(
                "Leave", [Op(OpCode.Jmp, Int(9)), Op(OpCode.Return, Int(1))], flags: 0x01));

            Assert.Equal(DataType.Void, runner.CallGlobal("Leave").Type);
        }

        [Fact]
        public void Return_NotTheLastInstruction_DoesNotBreakLuaSyntaxAndEndsTheFunction()
        {
            var runner = LuaRunner.LoadFunction(Function(
                "TwoReturns", [Op(OpCode.Return, Int(1)), Op(OpCode.Return, Int(2))], returnType: "Int", flags: 0x01));

            Assert.Equal(1, runner.CallGlobal("TwoReturns").Number);
        }

        [Fact]
        public void StrCat_ConvertsOperandsToStrings()
        {
            var runner = LuaRunner.LoadFunction(Function(
                "Cat", [Op(OpCode.StrCat, Id("s"), Id("a"), Id("b")), Op(OpCode.Return, Id("s"))],
                [("a", "String"), ("b", "Int")], [("s", "String")], "String", flags: 0x01));

            Assert.Equal("id 42", runner.CallGlobal("Cat", "id ", 42).String);
        }

        [Fact]
        public void Assign_LiteralOperands_AreTypedValues()
        {
            var runner = LuaRunner.LoadFunction(Function(
                "Lit",
                [
                    Op(OpCode.Assign, Id("i"), Int(-3)),
                    Op(OpCode.Assign, Id("f"), Float(2.5f)),
                    Op(OpCode.Assign, Id("s"), Str("q\"uote")),
                    Op(OpCode.Assign, Id("b"), Bool(true)),
                    Op(OpCode.Assign, Id("n"), None),
                    Op(OpCode.Return, Id("s")),
                ],
                null, [("i", "Int"), ("f", "Float"), ("s", "String"), ("b", "Bool"), ("n", "Form")], "String", flags: 0x01));

            Assert.Equal("q\"uote", runner.CallGlobal("Lit").String);
        }

        [Fact]
        public void Identifier_IsResolvedCaseInsensitivelyLikePapyrus()
        {
            var runner = LuaRunner.LoadFunction(Function(
                "Run", [Op(OpCode.IAdd, Id("RESULT"), Id("A"), Id("b")), Op(OpCode.Return, Id("Result"))], AB, Result, "Int", flags: 0x01));

            Assert.Equal(9, runner.CallGlobal("Run", 4, 5).Number);
        }

        [Fact]
        public void Identifier_NotAParameterOrLocal_IsAScriptVariableOnSelf()
        {
            var function = Function("Bump", [Op(OpCode.IAdd, Id("count"), Id("count"), Int(1)), Op(OpCode.Return, Id("count"))], returnType: "Int");
            var runner = LuaRunner.LoadFunction(function);
            var self = runner.NewObject();
            self["count"] = 41;

            var result = runner.Call(self, "Bump");

            Assert.Equal(42, result.Number);
            Assert.Equal(42, self.Get("count").Number);
        }

        [Fact]
        public void Parameter_NamedLikeALuaKeyword_StillCompilesAndWorks()
        {
            var runner = LuaRunner.LoadFunction(Function(
                "Run", [Op(OpCode.Return, Id("end"))], [("end", "Int")], flags: 0x01));

            Assert.Equal(5, runner.CallGlobal("Run", 5).Number);
        }

        [Fact]
        public void CallMethod_VoidResult_CallsTheMethodOnTheReceiverWithArguments()
        {
            var function = Function(
                "Run",
                [Op(OpCode.CallMethod, Str("Notify"), Id("target"), Id("::NoneVar"), Int(2), Id("a"), Str("tag"))],
                [("target", "Form"), ("a", "Int")], flags: 0x01);
            var runner = LuaRunner.LoadFunction(function);
            var target = runner.NewTable();
            DynValue[] received = [];
            target["Notify"] = DynValue.NewCallback((_, args) =>
            {
                received = args.GetArray();
                return DynValue.Nil;
            });

            runner.CallGlobal("Run", target, 9);

            Assert.Same(target, received[0].Table);
            Assert.Equal(9, received[1].Number);
            Assert.Equal("tag", received[2].String);
            Assert.Equal(3, received.Length);
        }

        [Fact]
        public void CallMethod_WithResult_AssignsTheReturnValue()
        {
            var function = Function(
                "Run",
                [Op(OpCode.CallMethod, Str("Double"), Id("target"), Id("r"), Int(1), Id("x")), Op(OpCode.Return, Id("r"))],
                [("target", "Form"), ("x", "Int")], Result.Select(l => ("r", l.Item2)).ToArray(), "Int", flags: 0x01);
            var runner = LuaRunner.LoadFunction(function);
            var target = runner.NewTable();
            target["Double"] = DynValue.NewCallback((_, args) => DynValue.NewNumber(args[1].Number * 2));

            Assert.Equal(14, runner.CallGlobal("Run", target, 7).Number);
        }

        [Fact]
        public void CallMethod_NameThatIsALuaKeyword_UsesBracketCallWithTheReceiverAsFirstArgument()
        {
            var function = Function(
                "Run",
                [Op(OpCode.CallMethod, Str("end"), Id("target"), Id("r"), Int(0)), Op(OpCode.Return, Id("r"))],
                [("target", "Form")], [("r", "Int")], "Int", flags: 0x01);
            var runner = LuaRunner.LoadFunction(function);
            var target = runner.NewTable();
            target["end"] = DynValue.NewCallback((_, args) => DynValue.NewBoolean(args[0].Table == target));

            Assert.True(runner.CallGlobal("Run", target).Boolean);
        }

        [Fact]
        public void PropertySetThenGet_ThroughAutoProperty_StoresInTheBackingVariable()
        {
            var property = new PexProperty("Health", "Int", "", 0, 0x07, "::Health_var", null, null);
            var script = Script("Actor", [new PexState("", [
                Function("Run", [
                    Op(OpCode.PropSet, Str("Health"), Id("self"), Int(75)),
                    Op(OpCode.PropGet, Str("Health"), Id("self"), Id("h")),
                    Op(OpCode.Return, Id("h")),
                ], null, [("h", "Int")], "Int"),
            ])], properties: [property]);
            var runner = LuaRunner.Load(File(script));
            var self = runner.NewObject();

            var result = runner.Call(self, "Run");

            Assert.Equal(75, result.Number);
            Assert.Equal(75, self.Get("::Health_var").Number);
        }

        [Fact]
        public void PropertyWithHandlers_RunsTheDeclaredGetterAndSetter()
        {
            var getter = Function("", [Op(OpCode.Return, Id("::Stored"))], returnType: "Int");
            var setter = Function("", [Op(OpCode.IMul, Id("::Stored"), Id("value"), Int(2))], [("value", "Int")]);
            var property = new PexProperty("Doubled", "Int", "", 0, 0x03, "", getter, setter);
            var runner = LuaRunner.Load(File(Script("S", [new PexState("", [])], properties: [property])));
            var self = runner.NewObject();

            runner.Call(self, "set_Doubled", 21);

            Assert.Equal(42, runner.Call(self, "get_Doubled").Number);
        }

        [Fact]
        public void Struct_SetThenGetMember_RoundTripsThroughAKeyedTable()
        {
            var runner = LuaRunner.LoadFunction(Function(
                "Run",
                [
                    Op(OpCode.StructSet, Id("s"), Str("Count"), Int(12)),
                    Op(OpCode.StructGet, Id("r"), Id("s"), Str("Count")),
                    Op(OpCode.Return, Id("r")),
                ],
                [("s", "Pair")], [("r", "Int")], "Int", flags: 0x01));
            var structure = runner.NewTable();

            var result = runner.CallGlobal("Run", structure);

            Assert.Equal(12, result.Number);
            Assert.Equal(12, structure.Get("Count").Number);
        }

        [Fact]
        public void StructCreate_UsesTheDeclaredTypeOfTheDestination()
        {
            string lua = EmitBody(Op(OpCode.StructCreate, Id("s")));

            Assert.Contains("s = _papyrus.struct_create(\"Pair\")", lua);
        }

        [Fact]
        public void Cast_UsesTheDeclaredTypeOfTheDestination()
        {
            string lua = EmitBody(Op(OpCode.Cast, Id("d"), Id("a")));

            Assert.Contains("d = _papyrus.cast(a, \"Int\")", lua);
        }

        [Fact]
        public void ArrayElements_ZeroBasedPapyrusIndexesMapToOneBasedLuaTables()
        {
            var runner = LuaRunner.LoadFunction(Function(
                "Run",
                [
                    Op(OpCode.ArrayGetElement, Id("first"), Id("arr"), Int(0)),
                    Op(OpCode.ArraySetElement, Id("arr"), Int(2), Int(99)),
                    Op(OpCode.ArrayGetElement, Id("last"), Id("arr"), Int(2)),
                    Op(OpCode.ArrayLength, Id("len"), Id("arr")),
                    Op(OpCode.IAdd, Id("sum"), Id("first"), Id("last")),
                    Op(OpCode.IAdd, Id("sum"), Id("sum"), Id("len")),
                    Op(OpCode.Return, Id("sum")),
                ],
                [("arr", "Int[]")], [("first", "Int"), ("last", "Int"), ("len", "Int"), ("sum", "Int")], "Int", flags: 0x01));
            var array = runner.NewTable(10, 20, 30);

            var result = runner.CallGlobal("Run", array);

            Assert.Equal(10 + 99 + 3, result.Number);
            Assert.Equal(99, array.Get(3).Number);
        }

        [Fact]
        public void ArrayInsertAndRemoveLast_ShiftFromZeroBasedIndexes()
        {
            var runner = LuaRunner.LoadFunction(Function(
                "Run",
                [
                    Op(OpCode.ArrayInsert, Id("arr"), Int(5), Int(1)),
                    Op(OpCode.ArrayRemoveLast, Id("arr")),
                ],
                [("arr", "Int[]")], flags: 0x01));
            var array = runner.NewTable(10, 20, 30);

            runner.CallGlobal("Run", array);

            Assert.Equal(3, array.Length);
            Assert.Equal(new[] { 10.0, 5.0, 20.0 }, new[] { array.Get(1).Number, array.Get(2).Number, array.Get(3).Number });
        }

        [Theory]
        [InlineData(OpCode.ArrayCreate, "_papyrus.array_create(")]
        [InlineData(OpCode.ArrayClear, "_papyrus.array_clear(")]
        [InlineData(OpCode.ArrayAdd, "_papyrus.array_add(")]
        [InlineData(OpCode.ArrayRemove, "_papyrus.array_remove(")]
        [InlineData(OpCode.ArrayFindElement, "_papyrus.array_find(")]
        [InlineData(OpCode.ArrayRFindElement, "_papyrus.array_rfind(")]
        [InlineData(OpCode.ArrayFindStruct, "_papyrus.array_find_struct(")]
        [InlineData(OpCode.ArrayRFindStruct, "_papyrus.array_rfind_struct(")]
        [InlineData(OpCode.Cast, "_papyrus.cast(")]
        [InlineData(OpCode.Is, "_papyrus.is_type(")]
        [InlineData(OpCode.LockGuards, "_papyrus.lock_guards(")]
        [InlineData(OpCode.UnlockGuards, "_papyrus.unlock_guards(")]
        [InlineData(OpCode.TryLockGuards, "_papyrus.try_lock_guards(")]
        public void RuntimeBackedOpcodes_CallTheDocumentedRuntimeHelper(OpCode op, string helperCall)
        {
            string lua = LuaEmitter.Emit(File(Script("S", Function("F", [InstructionFor(op)], null, Locals, flags: 0x01))));

            Assert.Contains(helperCall, lua);
            LuaRunner.AssertCompiles(lua);
        }

        [Fact]
        public void CallStatic_EmitsLookupInTheScriptRegistry()
        {
            string lua = EmitBody(Op(OpCode.CallStatic, Str("Game"), Str("GetPlayer"), Id("o"), Int(1), Int(3)));

            Assert.Contains("o = _papyrus.scripts[\"Game\"].GetPlayer(3)", lua);
        }

        [Fact]
        public void CallParent_PassesSelfAsFirstArgument()
        {
            string withArgs = EmitBody(Op(OpCode.CallParent, Str("OnInit"), Id("::NoneVar"), Int(1), Int(3)));
            string withoutArgs = EmitBody(Op(OpCode.CallParent, Str("OnInit"), Id("::NoneVar"), Int(0)));

            Assert.Contains("_papyrus.parent(self).OnInit(self, 3)", withArgs);
            Assert.Contains("_papyrus.parent(self).OnInit(self)", withoutArgs);
        }

        [Fact]
        public void CallParent_WithLuaKeywordMethodName_IndexesByStringKey()
        {
            string lua = EmitBody(Op(OpCode.CallParent, Str("end"), Id("::NoneVar"), Int(0)));

            Assert.Contains("_papyrus.parent(self)[\"end\"](self)", lua);
            LuaRunner.AssertCompiles(lua);
        }

        [Fact]
        public void PropGetAndSet_WithLuaKeywordPropertyName_EmitCompilableAccessors()
        {
            string lua = EmitBody(
                Op(OpCode.PropSet, Str("end"), Id("self"), Int(1)),
                Op(OpCode.PropGet, Str("end"), Id("self"), Id("d")));

            Assert.Contains("self:set_end(1)", lua);
            Assert.Contains("d = self:get_end()", lua);
            LuaRunner.AssertCompiles(lua);
        }

        [Fact]
        public void PropGetAndSet_WithNonIdentifierPropertyName_IndexByStringKeyAndStayCompilable()
        {
            string lua = EmitBody(
                Op(OpCode.PropSet, Str("my prop"), Id("self"), Int(1)),
                Op(OpCode.PropGet, Str("my prop"), Id("self"), Id("d")));

            Assert.Contains("self[\"set_my prop\"](self, 1)", lua);
            Assert.Contains("d = self[\"get_my prop\"](self)", lua);
            LuaRunner.AssertCompiles(lua);
        }

        [Fact]
        public void CallMethod_NoneVarDestinationInAnyCase_EmitsAStatementWithoutAssignment()
        {
            string lua = EmitBody(Op(OpCode.CallMethod, Str("Go"), Id("self"), Id("::nonevar"), Int(0)));

            Assert.Contains("self:Go()", lua);
            Assert.DoesNotContain("= self:Go()", lua);
        }

        [Fact]
        public void LowerFunction_EveryOpcodeInOneBody_ProducesCompilableLua()
        {
            var instructions = Enum.GetValues<OpCode>().Select(InstructionFor).ToArray();
            var function = Function("Everything", instructions, [("a", "Int"), ("b", "Int")], Locals, flags: 0x01);

            string lua = LuaEmitter.Emit(File(Script("S", function)));

            LuaRunner.AssertCompiles(lua);
            Assert.DoesNotContain("TODO: opcode", lua);
        }

        private static readonly (string, string)[] Locals =
        [
            ("a", "Int"), ("b", "Int"), ("d", "Int"), ("arr", "Int[]"), ("o", "Form"), ("s", "Pair"), ("t", "Bool"),
        ];

        private static PexInstruction InstructionFor(OpCode op) => op switch
        {
            OpCode.Nop => Op(op),
            OpCode.IAdd or OpCode.FAdd or OpCode.ISub or OpCode.FSub or OpCode.IMul or OpCode.FMul or OpCode.IDiv or OpCode.FDiv or OpCode.IMod
                or OpCode.CmpEq or OpCode.CmpLt or OpCode.CmpLte or OpCode.CmpGt or OpCode.CmpGte or OpCode.StrCat => Op(op, Id("d"), Id("a"), Id("b")),
            OpCode.Not or OpCode.INeg or OpCode.FNeg or OpCode.Assign or OpCode.Cast => Op(op, Id("d"), Id("a")),
            OpCode.Jmp => Op(op, Int(1)),
            OpCode.JmpT or OpCode.JmpF => Op(op, Id("t"), Int(1)),
            OpCode.CallMethod => Op(op, Str("M"), Id("o"), Id("d"), Int(1), Id("a")),
            OpCode.CallParent => Op(op, Str("M"), Id("d"), Int(1), Id("a")),
            OpCode.CallStatic => Op(op, Str("G"), Str("M"), Id("d"), Int(1), Id("a")),
            OpCode.Return => Op(op, Id("d")),
            OpCode.PropGet => Op(op, Str("P"), Id("o"), Id("d")),
            OpCode.PropSet => Op(op, Str("P"), Id("o"), Id("a")),
            OpCode.ArrayCreate => Op(op, Id("arr"), Int(4)),
            OpCode.ArrayLength => Op(op, Id("d"), Id("arr")),
            OpCode.ArrayGetElement => Op(op, Id("d"), Id("arr"), Int(0)),
            OpCode.ArraySetElement => Op(op, Id("arr"), Int(0), Id("a")),
            OpCode.ArrayFindElement or OpCode.ArrayRFindElement => Op(op, Id("arr"), Id("d"), Id("a"), Int(0)),
            OpCode.Is => Op(op, Id("t"), Id("o"), Str("Form")),
            OpCode.StructCreate => Op(op, Id("s")),
            OpCode.StructGet => Op(op, Id("d"), Id("s"), Str("m")),
            OpCode.StructSet => Op(op, Id("s"), Str("m"), Id("a")),
            OpCode.ArrayFindStruct or OpCode.ArrayRFindStruct => Op(op, Id("arr"), Id("d"), Str("m"), Id("a"), Int(0)),
            OpCode.ArrayAdd => Op(op, Id("arr"), Id("a"), Int(1)),
            OpCode.ArrayInsert => Op(op, Id("arr"), Id("a"), Int(0)),
            OpCode.ArrayRemoveLast or OpCode.ArrayClear => Op(op, Id("arr")),
            OpCode.ArrayRemove => Op(op, Id("arr"), Int(0), Int(1)),
            OpCode.LockGuards or OpCode.UnlockGuards => Op(op, Int(2), Id("o"), Id("o")),
            OpCode.TryLockGuards => Op(op, Id("t"), Int(1), Id("o")),
            _ => throw new ArgumentOutOfRangeException(nameof(op), op, "no sample instruction"),
        };

        private static string EmitBody(params PexInstruction[] instructions) =>
            LuaEmitter.Emit(File(Script("S", Function("F", instructions, null, Locals))));
    }
}
