using AssetConverter.PapyrusLua.Pex;
using Xunit;

namespace InstallerTests
{
    // Independent source: the operand layouts of the documented PEX opcode set, written out below.
    public class OpcodeTableTests
    {
        private sealed record OpcodeLayout(int Id, string Name, int FixedOperands, bool VarArgs);

        [Fact]
        public void Specs_EveryOpcode_HasTheSameIdMnemonicAndOperandLayout()
        {
            var layouts = ExpectedLayouts;
            Assert.Equal(50, layouts.Length);

            Assert.All(layouts, row =>
            {
                var spec = OpCodeTable.Get((OpCode)row.Id);
                Assert.Equal(row.Name, ((OpCode)row.Id).ToString());
                Assert.Equal(row.Name.ToLowerInvariant(), spec.Mnemonic.Replace("_", ""));
                Assert.Equal(row.FixedOperands, spec.FixedOperands);
                Assert.Equal(row.VarArgs ? OpCodeArity.VarArgs : OpCodeArity.Fixed, spec.Arity);
            });
        }

        [Theory]
        [InlineData(50)]
        [InlineData(255)]
        public void Get_OpcodeBeyondTheFo4Set_ThrowsInvalidData(int code)
        {
            var ex = Assert.Throws<InvalidDataException>(() => OpCodeTable.Get((OpCode)code));

            Assert.Contains($"0x{code:X2}", ex.Message);
        }

        private static readonly OpcodeLayout[] ExpectedLayouts =
        [
            new(0, "Nop", 0, false),
            new(1, "IAdd", 3, false),
            new(2, "FAdd", 3, false),
            new(3, "ISub", 3, false),
            new(4, "FSub", 3, false),
            new(5, "IMul", 3, false),
            new(6, "FMul", 3, false),
            new(7, "IDiv", 3, false),
            new(8, "FDiv", 3, false),
            new(9, "IMod", 3, false),
            new(10, "Not", 2, false),
            new(11, "INeg", 2, false),
            new(12, "FNeg", 2, false),
            new(13, "Assign", 2, false),
            new(14, "Cast", 2, false),
            new(15, "CmpEq", 3, false),
            new(16, "CmpLt", 3, false),
            new(17, "CmpLte", 3, false),
            new(18, "CmpGt", 3, false),
            new(19, "CmpGte", 3, false),
            new(20, "Jmp", 1, false),
            new(21, "JmpT", 2, false),
            new(22, "JmpF", 2, false),
            new(23, "CallMethod", 3, true),
            new(24, "CallParent", 2, true),
            new(25, "CallStatic", 3, true),
            new(26, "Return", 1, false),
            new(27, "StrCat", 3, false),
            new(28, "PropGet", 3, false),
            new(29, "PropSet", 3, false),
            new(30, "ArrayCreate", 2, false),
            new(31, "ArrayLength", 2, false),
            new(32, "ArrayGetElement", 3, false),
            new(33, "ArraySetElement", 3, false),
            new(34, "ArrayFindElement", 4, false),
            new(35, "ArrayRFindElement", 4, false),
            new(36, "Is", 3, false),
            new(37, "StructCreate", 1, false),
            new(38, "StructGet", 3, false),
            new(39, "StructSet", 3, false),
            new(40, "ArrayFindStruct", 5, false),
            new(41, "ArrayRFindStruct", 5, false),
            new(42, "ArrayAdd", 3, false),
            new(43, "ArrayInsert", 3, false),
            new(44, "ArrayRemoveLast", 1, false),
            new(45, "ArrayRemove", 3, false),
            new(46, "ArrayClear", 1, false),
            new(47, "LockGuards", 0, true),
            new(48, "UnlockGuards", 0, true),
            new(49, "TryLockGuards", 1, true)
        ];
    }
}
