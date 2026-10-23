using AssetConverter.PapyrusLua.Lua;
using Xunit;

namespace InstallerTests
{
    public class LuaWriterTests
    {
        [Fact]
        public void WriteLine_AfterIndent_PrefixesFourSpacesPerLevel()
        {
            var writer = new LuaWriter();

            writer.WriteLine("a");
            writer.Indent();
            writer.WriteLine("b");
            writer.Indent();
            writer.WriteLine("c");
            writer.Dedent();
            writer.WriteLine("d");
            writer.Dedent();
            writer.WriteLine("e");

            Assert.Equal("a\n    b\n        c\n    d\ne\n", writer.ToString());
        }

        [Fact]
        public void WriteLine_BlankLineInsideIndentation_HasNoTrailingWhitespace()
        {
            var writer = new LuaWriter();
            writer.Indent();

            writer.WriteLine();

            Assert.Equal("\n", writer.ToString());
        }

        [Fact]
        public void Dedent_AtTopLevel_Throws()
        {
            var writer = new LuaWriter();

            Assert.Throws<InvalidOperationException>(writer.Dedent);
        }
    }
}
