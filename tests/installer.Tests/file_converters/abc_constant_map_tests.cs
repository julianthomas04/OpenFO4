using AssetConverter.Abc;
using Xunit;

namespace InstallerTests
{
    public sealed class AbcConstantMapTests
    {
        private static AbcConstantMap Extract(params AbcBuilder[] blocks) =>
            AbcConstantExtractor.Extract(blocks.Select((b, i) => ($"block{i}", AbcReader.Read(b.Build()))).ToList(), "Test");

        /// <summary>A class <c>Pkg.Widget</c> with the given static and instance traits and the bodies of its two initialisers.</summary>
        private static AbcBuilder Widget(
            Func<AbcBuilder, IEnumerable<AbcTraitSpec>>? classTraits = null,
            Func<AbcBuilder, IEnumerable<AbcTraitSpec>>? instanceTraits = null,
            Func<AbcBuilder, AbcCode, AbcCode>? classInit = null,
            Func<AbcBuilder, AbcCode, AbcCode>? instanceInit = null,
            AbcBuilder? into = null)
        {
            var abc = into ?? new AbcBuilder();
            int cinit = classInit is null ? abc.Method() : abc.Function(classInit(abc, new AbcCode().GetLocal0().PushScope()).ReturnVoid().ToArray());
            int iinit = instanceInit is null ? abc.Method() : abc.Function(instanceInit(abc, new AbcCode().GetLocal0().PushScope()).ReturnVoid().ToArray());
            abc.Class("Pkg", "Widget", 0, iinit, cinit, instanceTraits?.Invoke(abc), classTraits?.Invoke(abc));
            return abc;
        }

        /// <summary>A <c>Pkg.Widget</c> whose method <paramref name="name"/> runs <paramref name="body"/>.</summary>
        private static AbcBuilder WidgetMethod(string name, Func<AbcBuilder, AbcCode, AbcCode> body, int kind = 1, int parameters = 0)
        {
            var abc = new AbcBuilder();
            int method = abc.Function(body(abc, new AbcCode().GetLocal0().PushScope()).ReturnVoid().ToArray(), parameters, localCount: parameters + 2);
            return Widget(instanceTraits: a => [new AbcMethodTraitSpec(a.Name(name), method, kind)], into: abc);
        }

        private static AbcSlotSpec Const(AbcBuilder abc, string name, int value) => new(abc.Name(name), IsConst: true, abc.Int(value), AbcBuilder.ConstantInt);

        private static AbcSlotSpec Slot(AbcBuilder abc, string name) => new(abc.Name(name));

        [Fact]
        public void ReferenceLists_ArrayLiteralOfLookedUpClasses_IsTheListOfTheirQualifiedNamesInOrder()
        {
            var map = Extract(Widget(
                classTraits: a => [Slot(a, "KINDS")],
                classInit: (a, c) => c.FindProperty(a.Name("KINDS")).GetLex(a.Name("Beta", "Pkg")).GetLex(a.Name("Alpha", "Pkg")).Op(0x56, 2).InitProperty(a.Name("KINDS"))));

            Assert.Equal(["Pkg.Beta", "Pkg.Alpha"], map.ReferenceLists["Pkg.Widget.KINDS"]);
            Assert.DoesNotContain("Pkg.Widget.KINDS", map.Constants.Keys);
        }

        [Fact]
        public void ReferenceLists_ArrayMixingConstantsAndReferences_IsNotAList()
        {
            var map = Extract(Widget(
                classTraits: a => [Slot(a, "KINDS")],
                classInit: (a, c) => c.FindProperty(a.Name("KINDS")).GetLex(a.Name("Alpha", "Pkg")).PushByte(3).Op(0x56, 2).InitProperty(a.Name("KINDS"))));

            Assert.DoesNotContain("Pkg.Widget.KINDS", map.ReferenceLists.Keys);
        }

        [Fact]
        public void ReferenceLists_ListAssignedTwice_IsNotAList()
        {
            var map = Extract(Widget(
                classTraits: a => [Slot(a, "KINDS")],
                classInit: (a, c) => c
                    .FindProperty(a.Name("KINDS")).GetLex(a.Name("Alpha", "Pkg")).Op(0x56, 1).InitProperty(a.Name("KINDS"))
                    .FindProperty(a.Name("KINDS")).GetLex(a.Name("Beta", "Pkg")).Op(0x56, 1).InitProperty(a.Name("KINDS"))));

            Assert.DoesNotContain("Pkg.Widget.KINDS", map.ReferenceLists.Keys);
        }

        [Fact]
        public void Constants_StatedDefaultOfAConst_IsItsValue()
        {
            var map = Extract(Widget(classTraits: a => [Const(a, "LIMIT", 42), new AbcSlotSpec(a.Name("RATE"), true, a.Double(0.5), AbcBuilder.ConstantDouble)]));

            Assert.Equal(42, map.Constants["Pkg.Widget.LIMIT"]);
            Assert.Equal(0.5, map.Constants["Pkg.Widget.RATE"]);
        }

        [Fact]
        public void Constants_UnconditionalAssignmentInTheInitialiser_IsTheValue()
        {
            var map = Extract(Widget(
                classTraits: a => [Slot(a, "SIZE")],
                classInit: (a, c) => c.FindProperty(a.Name("SIZE")).PushByte(9).InitProperty(a.Name("SIZE"))));

            Assert.Equal(9, map.Constants["Pkg.Widget.SIZE"]);
        }

        [Fact]
        public void Constants_InstanceFieldSetInTheConstructor_IsKeyedOnTheClass()
        {
            var map = Extract(Widget(
                instanceTraits: a => [Slot(a, "speed")],
                instanceInit: (a, c) => c.GetLocal0().PushDouble(a.Double(1.5)).InitProperty(a.Name("speed"))));

            Assert.Equal(1.5, map.Constants["Pkg.Widget.speed"]);
        }

        [Fact]
        public void Constants_DefaultAndAnEqualAssignment_AreOneValue()
        {
            var map = Extract(Widget(
                classTraits: a => [new AbcSlotSpec(a.Name("ALPHA"), true, a.Double(1), AbcBuilder.ConstantDouble)],
                classInit: (a, c) => c.FindProperty(a.Name("ALPHA")).PushByte(1).InitProperty(a.Name("ALPHA"))));

            Assert.Equal(1.0, map.Constants["Pkg.Widget.ALPHA"]);
        }

        [Fact]
        public void Constants_DefaultAndADifferentAssignment_AreUnresolved()
        {
            var map = Extract(Widget(
                classTraits: a => [Const(a, "ALPHA", 1)],
                classInit: (a, c) => c.FindProperty(a.Name("ALPHA")).PushByte(2).InitProperty(a.Name("ALPHA"))));

            Assert.Equal("assigned 2 times", map.Unresolved["Pkg.Widget.ALPHA"]);
            Assert.False(map.Constants.ContainsKey("Pkg.Widget.ALPHA"));
        }

        [Fact]
        public void Constants_ConditionalAssignment_IsUnresolved()
        {
            var map = Extract(Widget(
                classTraits: a => [Slot(a, "MODE")],
                classInit: (a, c) =>
                {
                    c.GetLex(a.Name("flag"));
                    int skip = c.IfFalse();
                    c.FindProperty(a.Name("MODE")).PushByte(3).InitProperty(a.Name("MODE"));
                    return c.Here(skip);
                }));

            Assert.Equal("assigned conditionally", map.Unresolved["Pkg.Widget.MODE"]);
        }

        [Fact]
        public void Constants_ComputedAssignment_IsUnresolved()
        {
            var map = Extract(Widget(
                classTraits: a => [Slot(a, "WIDTH")],
                classInit: (a, c) => c.FindProperty(a.Name("WIDTH")).GetLex(a.Name("stageWidth")).InitProperty(a.Name("WIDTH"))));

            Assert.Equal("computed from an expression", map.Unresolved["Pkg.Widget.WIDTH"]);
        }

        [Fact]
        public void Constants_DeclaredWithoutAValue_IsUnresolved()
        {
            Assert.Equal("declared without a value", Extract(Widget(instanceTraits: a => [Slot(a, "child_mc")])).Unresolved["Pkg.Widget.child_mc"]);
        }

        [Fact]
        public void Constants_FoldedFromConstantOperands_IsTheResultAndMarkedFolded()
        {
            var map = Extract(Widget(
                classTraits: a => [Slot(a, "AREA")],
                classInit: (a, c) => c.FindProperty(a.Name("AREA")).PushByte(6).PushByte(7).Multiply().InitProperty(a.Name("AREA"))));

            Assert.Equal(42, Convert.ToInt32(map.Constants["Pkg.Widget.AREA"], System.Globalization.CultureInfo.InvariantCulture));
            Assert.Contains("Pkg.Widget.AREA", map.FoldedConstants);
        }

        [Fact]
        public void Literals_PropertyAssignmentsInAMethod_AreKeyedByTheirTargetInBytecodeOrder()
        {
            var map = Extract(WidgetMethod("Refresh", (a, c) =>
            {
                c.GetLocal0().GetProperty(a.Name("border")).PushDouble(a.Double(0.35)).SetProperty(a.Name("alpha"));
                return c.GetLocal0().GetProperty(a.Name("border")).PushByte(0).SetProperty(a.Name("alpha"));
            }));

            Assert.Equal([0.35, 0], map.Literals["Pkg.Widget.Refresh#border.alpha="].Values);
        }

        [Fact]
        public void Literals_CallArguments_AreKeyedByCalleeAndArgumentIndex()
        {
            var map = Extract(WidgetMethod("frame95", (a, c) =>
                c.FindPropStrict(a.Name("gotoAndPlay")).PushByte(2).CallPropVoid(a.Name("gotoAndPlay"), 1)
                 .FindPropStrict(a.Name("fade")).PushString(a.String("in")).PushShort(-300).CallPropVoid(a.Name("fade"), 2)));

            Assert.Equal([2], map.Literals["Pkg.Widget.frame95#gotoAndPlay(0)"].Values);
            Assert.Equal(["in"], map.Literals["Pkg.Widget.frame95#fade(0)"].Values);
            Assert.Equal([-300], map.Literals["Pkg.Widget.frame95#fade(1)"].Values);
        }

        [Fact]
        public void Literals_BothArmsOfAConditional_AreListedInBytecodeOrder()
        {
            var map = Extract(WidgetMethod("Paint", (a, c) =>
            {
                c.GetLocal0().GetProperty(a.Name("label")).GetLocal0().GetProperty(a.Name("selected"));
                int otherwise = c.IfFalse();
                c.PushInt(a.Int(0x112233));
                int end = c.Jump();
                c.Here(otherwise).PushInt(a.Int(0x445566));
                return c.Here(end).SetProperty(a.Name("textColor"));
            }));

            Assert.Equal([0x112233, 0x445566], map.Literals["Pkg.Widget.Paint#label.textColor="].Values);
        }

        [Fact]
        public void Literals_FoldedExpression_IsOneValueMarkedFolded()
        {
            var map = Extract(WidgetMethod("Layout", (a, c) =>
                c.GetLocal0().PushByte(10).Negate().PushByte(4).Add().SetProperty(a.Name("x"))));

            var list = map.Literals["Pkg.Widget.Layout#x="];
            Assert.Equal([-6], list.Values.Select(v => Convert.ToInt32(v, System.Globalization.CultureInfo.InvariantCulture)));
            Assert.Equal([0], list.Folded);
        }

        [Fact]
        public void Literals_OperandOfAComputation_IsKeyedByTheOperation()
        {
            var map = Extract(WidgetMethod("Shrink", (a, c) =>
                c.GetLocal0().GetLex(a.Name("LIMIT")).PushByte(3).Subtract().SetProperty(a.Name("width"))));

            Assert.Equal([3], map.Literals["Pkg.Widget.Shrink#LIMIT-"].Values);
        }

        [Fact]
        public void Literals_UnreachableCode_IsIgnored()
        {
            var map = Extract(WidgetMethod("Done", (a, c) =>
            {
                c.ReturnVoid();
                return c.GetLocal0().PushByte(5).SetProperty(a.Name("dead"));
            }));

            Assert.DoesNotContain(map.Literals.Keys, k => k.Contains("dead", StringComparison.Ordinal));
        }

        [Fact]
        public void Literals_AccessorsAndClosures_AreNamedByTheirOwner()
        {
            var abc = new AbcBuilder();
            int closure = abc.Function(new AbcCode().GetLocal0().PushScope().GetLocal0().PushByte(4).SetProperty(abc.Name("depth")).ReturnVoid().ToArray());
            int getter = abc.Function(new AbcCode().GetLocal0().PushScope().NewFunction(closure).Pop().GetLocal0().PushByte(8).SetProperty(abc.Name("count")).ReturnVoid().ToArray());
            var map = Extract(Widget(instanceTraits: a => [new AbcMethodTraitSpec(a.Name("Size"), getter, Kind: 2)], into: abc));

            Assert.Equal([8], map.Literals["Pkg.Widget.get:Size#count="].Values);
            Assert.Equal([4], map.Literals["Pkg.Widget.get:Size/fn0#depth="].Values);
        }

        [Fact]
        public void Literals_MethodNoTraitNames_IsKeyedByItsIndex()
        {
            var abc = new AbcBuilder();
            int orphan = abc.Function(new AbcCode().GetLocal0().PushScope().GetLocal0().PushByte(1).SetProperty(abc.Name("x")).ReturnVoid().ToArray());

            var map = Extract(abc);

            Assert.Equal([1], map.Literals[$"<method{orphan}>#x="].Values);
        }

        [Fact]
        public void Keys_DefinedInTwoBlocks_AreUnresolved()
        {
            var map = Extract(Widget(classTraits: a => [Const(a, "LIMIT", 1)]), Widget(classTraits: a => [Const(a, "LIMIT", 1)]));

            Assert.Equal("defined in 2 ABC blocks", map.Unresolved["Pkg.Widget.LIMIT"]);
        }

        [Fact]
        public void SuperClasses_AreTheQualifiedNamesEachClassExtends()
        {
            var abc = new AbcBuilder();
            abc.Class("Lib", "Base", 0, abc.Method(), abc.Method(), [Const(abc, "fAlpha", 3)]);
            abc.Class("Lib", "Middle", abc.Name("Base", "Lib"), abc.Method(), abc.Method());
            abc.Class("", "Leaf", abc.Name("Middle", "Lib"), abc.Method(), abc.Method());

            var map = Extract(abc);

            Assert.Equal("Lib.Middle", map.SuperClasses["Leaf"]);
            Assert.Equal("Lib.Base", map.SuperClasses["Lib.Middle"]);
        }

        [Fact]
        public void Constants_AssignmentAfterABranch_IsUnconditional()
        {
            var map = Extract(Widget(
                classTraits: a => [Slot(a, "MODE"), Slot(a, "LEVEL")],
                classInit: (a, c) =>
                {
                    c.GetLex(a.Name("flag"));
                    int skip = c.IfFalse();
                    c.FindProperty(a.Name("MODE")).PushByte(3).InitProperty(a.Name("MODE"));
                    return c.Here(skip).FindProperty(a.Name("LEVEL")).PushByte(4).InitProperty(a.Name("LEVEL"));
                }));

            Assert.Equal(4, map.Constants["Pkg.Widget.LEVEL"]);
            Assert.Equal("assigned conditionally", map.Unresolved["Pkg.Widget.MODE"]);
        }

        [Fact]
        public void Constants_AssignmentAfterAGuardThatThrows_IsUnconditional()
        {
            var map = Extract(Widget(
                classTraits: a => [Slot(a, "LEVEL")],
                classInit: (a, c) =>
                {
                    c.GetLex(a.Name("broken"));
                    int fine = c.IfFalse();
                    c.PushString(a.String("broken")).Throw();
                    return c.Here(fine).FindProperty(a.Name("LEVEL")).PushByte(4).InitProperty(a.Name("LEVEL"));
                }));

            Assert.Equal(4, map.Constants["Pkg.Widget.LEVEL"]);
        }

        [Theory]
        [InlineData(0x70, "5")]
        [InlineData(0x76, "true")]
        [InlineData(0x75, "5")]
        public void Constants_ConversionOfAConstant_IsTheActionScriptResult(byte conversion, string expected)
        {
            var map = Extract(Widget(
                classTraits: a => [Slot(a, "VALUE")],
                classInit: (a, c) => c.FindProperty(a.Name("VALUE")).PushByte(5).Op(conversion).InitProperty(a.Name("VALUE"))));

            Assert.Equal(expected, AbcValues.ToActionScriptString(map.Constants["Pkg.Widget.VALUE"]));
            Assert.Contains("Pkg.Widget.VALUE", map.FoldedConstants);
        }

        [Theory]
        [InlineData("10", 10)]
        [InlineData("0x1F", 31)]
        [InlineData("", 0)]
        [InlineData("abc", 0)]
        public void Constants_StringConvertedToInt_IsTheActionScriptResult(string text, int expected)
        {
            var map = Extract(Widget(
                classTraits: a => [Slot(a, "VALUE")],
                classInit: (a, c) => c.FindProperty(a.Name("VALUE")).PushString(a.String(text)).Op(0x73).InitProperty(a.Name("VALUE"))));

            Assert.Equal(expected, map.Constants["Pkg.Widget.VALUE"]);
        }

        [Theory]
        [InlineData(" 10")]
        [InlineData("-0x10")]
        public void Constants_ConversionWithoutACertainResult_IsComputed(string text)
        {
            var map = Extract(Widget(
                classTraits: a => [Slot(a, "VALUE")],
                classInit: (a, c) => c.FindProperty(a.Name("VALUE")).PushString(a.String(text)).Op(0x73).InitProperty(a.Name("VALUE"))));

            Assert.Equal("computed from an expression", map.Unresolved["Pkg.Widget.VALUE"]);
        }

        [Theory]
        [InlineData(0.1, "0.1")]
        [InlineData(-2.5, "-2.5")]
        [InlineData(1e21, "1e+21")]
        [InlineData(123456789012345680000.0, "123456789012345680000")]
        [InlineData(0.000001, "0.000001")]
        [InlineData(1.5e-7, "1.5e-7")]
        [InlineData(100.0, "100")]
        public void NumberToString_FollowsTheEcmaScriptForm(double value, string expected)
        {
            Assert.Equal(expected, AbcValues.NumberToString(value));
        }

        [Fact]
        public void Literals_OperandOfAUnaryOperator_IsDescribed()
        {
            var map = Extract(WidgetMethod("Half", (a, c) =>
                c.GetLocal0().GetLocal(1).Negate().PushByte(2).Op(0xA3).SetProperty(a.Name("x")), parameters: 1));

            Assert.Equal([2], map.Literals["Pkg.Widget.Half#-($1)/"].Values);
        }

        [Fact]
        public void Literals_OperandThatCannotBeDescribed_IsUnresolved()
        {
            var map = Extract(WidgetMethod("Read", (a, c) =>
                c.GetLocal0().GetLocal(1).Op(0x35).PushByte(5).Add().SetProperty(a.Name("x")), parameters: 1));

            Assert.Equal("operand cannot be described", map.Unresolved["Pkg.Widget.Read#?+"]);
            Assert.DoesNotContain(map.Literals.Keys, k => k.StartsWith("Pkg.Widget.Read#", StringComparison.Ordinal) && k.Contains('+', StringComparison.Ordinal));
        }

        [Fact]
        public void Literals_ConditionalExpressionOperand_DescribesBothBranchesUnderItsCondition()
        {
            var map = Extract(WidgetMethod("Pick", (a, c) =>
            {
                c.GetLocal0().GetLocal(1);
                int otherwise = c.IfFalse();
                c.GetLocal(1);
                int end = c.Jump();
                c.Here(otherwise).GetLocal(2);
                return c.Here(end).PushByte(7).Add().SetProperty(a.Name("x"));
            }, parameters: 2));

            Assert.Equal([7], map.Literals["Pkg.Widget.Pick#($1?$1:$2)+"].Values);
        }

        [Fact]
        public void Literals_ConstantsChosenByAComparison_AreUsedByTheirConsumer()
        {
            var map = Extract(WidgetMethod("Clock", (a, c) =>
            {
                c.GetLocal0().GetLocal(1).PushByte(12);
                int otherwise = c.Branch(0x0C);
                c.PushString(a.String("AM"));
                int end = c.Jump();
                c.Here(otherwise).PushString(a.String("PM"));
                return c.Here(end).GetLocal(2).Add().SetProperty(a.Name("x"));
            }, parameters: 2));

            Assert.Equal(["AM", "PM"], map.Literals["Pkg.Widget.Clock#+$2"].Values);
            Assert.Equal([12], map.Literals["Pkg.Widget.Clock#$1<"].Values);
        }

        [Fact]
        public void Literals_ShortCircuitAnd_DescribesItsResult()
        {
            var map = Extract(WidgetMethod("Both", (a, c) =>
            {
                c.GetLocal0().GetLocal(1).Op(0x2A);
                int end = c.IfFalse();
                c.Pop().GetLocal(2);
                return c.Here(end).PushByte(3).Add().SetProperty(a.Name("x"));
            }, parameters: 2));

            Assert.Equal([3], map.Literals["Pkg.Widget.Both#($1?$2:$1)+"].Values);
        }

        [Fact]
        public void Literals_ValueChangedAroundALoop_AreUnresolved()
        {
            var map = Extract(WidgetMethod("Count", (a, c) =>
            {
                c.GetLocal0().GetLocal(1);
                int loop = c.Offset;
                c.Label().PushByte(1).Add().GetLocal(2);
                int done = c.IfFalse();
                c.JumpTo(loop);
                return c.Here(done).PushByte(7).Add().SetProperty(a.Name("x"));
            }, parameters: 2));

            Assert.Equal("operand cannot be described", map.Unresolved["Pkg.Widget.Count#?+"]);
        }

        [Fact]
        public void Constants_StaticSetByTheInstanceConstructor_IsUnresolved()
        {
            var map = Extract(Widget(
                classTraits: a => [Const(a, "COUNT", 1)],
                instanceInit: (a, c) => c.FindProperty(a.Name("COUNT")).PushByte(2).SetProperty(a.Name("COUNT"))));

            Assert.Equal("set by both the class and its instance initialiser", map.Unresolved["Pkg.Widget.COUNT"]);
            Assert.False(map.Constants.ContainsKey("Pkg.Widget.COUNT"));
        }

        [Fact]
        public void SuperClasses_ThatDisagreeBetweenBlocks_AreUnresolved()
        {
            var first = new AbcBuilder();
            first.Class("", "Leaf", first.Name("Base"), first.Method(), first.Method());
            var second = new AbcBuilder();
            second.Class("", "Leaf", second.Name("Other"), second.Method(), second.Method());

            var map = Extract(first, second);

            Assert.False(map.SuperClasses.ContainsKey("Leaf"));
            Assert.Equal("extends 2 different classes", map.Unresolved["Leaf"]);
        }

        [Fact]
        public void Extract_MalformedBytecode_Throws()
        {
            var abc = new AbcBuilder();
            abc.Function(new AbcCode().Raw(0xFF).ToArray());

            Assert.Throws<AbcFormatException>(() => Extract(abc));
        }

        [Fact]
        public void Extract_BranchIntoTheMiddleOfAnInstruction_Throws()
        {
            var abc = new AbcBuilder();
            abc.Function(new AbcCode().Raw(0x10, 0x01, 0x00, 0x00, 0x24, 0x05, 0x47).ToArray());

            Assert.Throws<AbcFormatException>(() => Extract(abc));
        }

        [Fact]
        public void Extract_StackThatRunsDry_Throws()
        {
            var abc = new AbcBuilder();
            abc.Function(new AbcCode().Pop().ReturnVoid().ToArray());

            Assert.Throws<AbcFormatException>(() => Extract(abc));
        }
    }
}
