using AssetConverter.PapyrusLua.Lua;
using AssetConverter.PapyrusLua.Pex;
using InstallerTests.Support;
using MoonSharp.Interpreter;
using Xunit;
using static InstallerTests.Support.PexModel;

namespace InstallerTests
{
    public class LuaEmitterTests
    {
        [Fact]
        public void Emit_Header_NamesTheSourceFileAndUtcCompileTime()
        {
            var pex = File() with { Header = PexBytes.Fo4Header("Quest.psc") with { CompileTime = 0 } };

            string lua = LuaEmitter.Emit(pex);

            Assert.StartsWith("-- Transpiled from Quest.psc\n-- PEX compile time: 1970-01-01 00:00:00Z\n", lua);
        }

        [Fact]
        public void Emit_ScriptWithParentConstAndAutoState_RecordsThemAsMetadata()
        {
            var script = new PexObject("Child", 0, "Parent", "", 1, 0, "Waiting", [], [], [], [new PexState("", [])]);

            string lua = LuaEmitter.Emit(File(script));

            Assert.Contains("__script.__papyrus_name = \"Child\"", lua);
            Assert.Contains("__script.__papyrus_extends = \"Parent\"", lua);
            Assert.Contains("__script.__papyrus_const = true", lua);
            Assert.Contains("__script.__papyrus_autoState = \"Waiting\"", lua);
        }

        [Fact]
        public void Emit_ScriptWithoutParent_OmitsExtendsConstAndAutoStateMetadata()
        {
            string lua = LuaEmitter.Emit(File(Script("Plain")));

            Assert.DoesNotContain("__papyrus_extends", lua);
            Assert.DoesNotContain("__papyrus_const", lua);
            Assert.DoesNotContain("__papyrus_autoState", lua);
        }

        [Fact]
        public void Emit_Module_ReturnsTheScriptTableWithDefaultStateRegistered()
        {
            var module = new Script().DoString(LuaEmitter.Emit(File(Script("Plain")))).Table;

            Assert.Equal("Plain", module.Get("__papyrus_name").String);
            Assert.Same(module, module.Get("__states").Table.Get("").Table);
            Assert.Same(module, module.Get("__index").Table);
        }

        [Fact]
        public void Emit_VariableInitialisers_AreTypedLuaLiterals()
        {
            var variables = new[]
            {
                new PexVariable("i", "Int", 0, Int(-4), 0),
                new PexVariable("f", "Float", 0, Float(3f), 0),
                new PexVariable("fr", "Float", 0, Float(0.25f), 0),
                new PexVariable("s", "String", 0, Str("a\"b\n"), 0),
                new PexVariable("b", "Bool", 0, Bool(true), 0),
                new PexVariable("n", "Form", 0, None, 1),
                new PexVariable("big", "Float", 0, Float(1e10f), 0),
            };
            var module = new Script().DoString(LuaEmitter.Emit(File(Script("V", [new PexState("", [])], variables)))).Table;

            Assert.Equal(-4, module.Get("i").Number);
            Assert.Equal(3, module.Get("f").Number);
            Assert.Equal(0.25, module.Get("fr").Number);
            Assert.Equal("a\"b\n", module.Get("s").String);
            Assert.True(module.Get("b").Boolean);
            Assert.True(module.Get("n").IsNil());
            Assert.Equal(1e10, module.Get("big").Number);
        }

        [Fact]
        public void Emit_FloatInitialiser_IsAlwaysWrittenAsAFloatLiteral()
        {
            string lua = LuaEmitter.Emit(File(Script("V", [new PexState("", [])], [new PexVariable("f", "Float", 0, Float(3f), 0)])));

            Assert.Contains("__script[\"f\"] = 3.0", lua);
        }

        [Theory]
        [InlineData(float.PositiveInfinity, "math.huge")]
        [InlineData(float.NegativeInfinity, "(-math.huge)")]
        [InlineData(float.NaN, "(0/0)")]
        public void Emit_NonFiniteFloatInitialiser_UsesLuaExpressions(float value, string expected)
        {
            string lua = LuaEmitter.Emit(File(Script("V", [new PexState("", [])], [new PexVariable("f", "Float", 0, Float(value), 0)])));

            Assert.Contains($"__script[\"f\"] = {expected}", lua);
            LuaRunner.AssertCompiles(lua);
        }

        [Fact]
        public void Emit_StructFactory_ReturnsAFreshTableWithInitialMembers()
        {
            var structs = new[] { new PexStruct("Pair", [new PexStructMember("Left", "Int", 0, Int(1), 0, ""), new PexStructMember("Right", "String", 0, Str("r"), 0, "")]) };
            var runner = LuaRunner.Load(File(Script("S", [new PexState("", [])], structs: structs)));
            var factory = runner.Module.Get("__struct_Pair");

            var first = runner.Invoke(factory).Table;
            var second = runner.Invoke(factory).Table;

            Assert.Equal(1, first.Get("Left").Number);
            Assert.Equal("r", first.Get("Right").String);
            Assert.NotSame(first, second);
        }

        [Fact]
        public void Emit_AutoProperty_GeneratesGetterAndSetterOverTheBackingVariable()
        {
            var property = new PexProperty("Count", "Int", "", 0, 0x07, "::Count_var", null, null);
            var runner = LuaRunner.Load(File(Script("S", [new PexState("", [])], properties: [property])));
            var self = runner.NewObject();

            runner.Call(self, "set_Count", 8);

            Assert.Equal(8, runner.Call(self, "get_Count").Number);
            Assert.Equal(8, self.Get("::Count_var").Number);
        }

        [Fact]
        public void Emit_NonDefaultState_PutsItsFunctionsInASeparateTableRegisteredByName()
        {
            var busy = new PexState("Busy State", [Function("Act", [Op(OpCode.Return, Int(2))], returnType: "Int")]);
            var idle = new PexState("", [Function("Act", [Op(OpCode.Return, Int(1))], returnType: "Int")]);
            var runner = LuaRunner.Load(File(Script("S", [idle, busy])));
            var busyTable = runner.Module.Get("__states").Table.Get("Busy State").Table;

            var idleResult = runner.Invoke(runner.Module.Get("Act"), runner.Module);
            var busyResult = runner.Invoke(busyTable.Get("Act"), runner.Module);

            Assert.Equal(1, idleResult.Number);
            Assert.Equal(2, busyResult.Number);
        }

        [Fact]
        public void Emit_NativeFunction_RaisesAnUnimplementedErrorWhenCalled()
        {
            var native = Function("Native", [], flags: 0x03);
            var runner = LuaRunner.LoadFunction(native);

            var ex = Assert.Throws<ScriptRuntimeException>(() => runner.CallGlobal("Native"));

            Assert.Contains("NATIVE_UNIMPLEMENTED", ex.DecoratedMessage);
        }

        [Fact]
        public void Emit_GlobalFunction_HasNoSelfParameterWhileMethodsDo()
        {
            var global = Function("Static", [Op(OpCode.Return, Id("x"))], [("x", "Int")], flags: 0x01);
            var method = Function("Method", [Op(OpCode.Return, Id("x"))], [("x", "Int")]);
            var runner = LuaRunner.Load(File(Script("S", global, method)));

            Assert.Equal(5, runner.CallGlobal("Static", 5).Number);
            Assert.Equal(5, runner.Call(runner.NewObject(), "Method", 5).Number);
        }

        [Fact]
        public void Emit_FunctionNameThatIsNotAnIdentifier_IsStoredUnderABracketKey()
        {
            var runner = LuaRunner.LoadFunction(Function("On Update", [Op(OpCode.Return, Int(3))], returnType: "Int", flags: 0x01));

            Assert.Equal(3, runner.CallGlobal("On Update").Number);
        }

        [Fact]
        public void Emit_LocalVariables_AreDeclaredNilAtTheTopOfTheFunction()
        {
            var function = Function("Run", [Op(OpCode.Return, Id("x"))], null, [("x", "Int")], "Int", flags: 0x01);

            string lua = LuaEmitter.Emit(File(Script("S", function)));

            Assert.Contains("local x  -- Int", lua);
        }

        [Fact]
        public void Emit_TwoScriptsInOneFile_EmitsTwoModulesInOrder()
        {
            string lua = LuaEmitter.Emit(File(Script("First"), Script("Second")));

            Assert.True(lua.IndexOf("-- Script: First", StringComparison.Ordinal) < lua.IndexOf("-- Script: Second", StringComparison.Ordinal));
            Assert.Equal(2, lua.Split("return __script").Length - 1);
        }
    }
}
