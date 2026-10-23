using AssetConverter.PapyrusLua.Lua;
using InstallerTests.Support;
using MoonSharp.Interpreter;
using Xunit;

namespace InstallerTests
{
    public class NameManglingTests
    {
        [Theory]
        [InlineData("abc", true)]
        [InlineData("_x1", true)]
        [InlineData("A_b_9", true)]
        [InlineData("", false)]
        [InlineData("1abc", false)]
        [InlineData("a b", false)]
        [InlineData("a::b", false)]
        [InlineData("caf\u00e9", false)]
        [InlineData("end", false)]
        [InlineData("goto", false)]
        [InlineData("nil", false)]
        [InlineData("End", true)]
        public void IsValidLuaIdentifier_Candidates_FollowLuaLexicalRules(string name, bool expected)
        {
            Assert.Equal(expected, NameMangling.IsValidLuaIdentifier(name));
        }

        [Theory]
        [InlineData("plain", "plain")]
        [InlineData("a b", "a_b")]
        [InlineData("::NoneVar", "__NoneVar")]
        [InlineData("2x", "_2x")]
        [InlineData("end", "_end")]
        [InlineData("", "_")]
        [InlineData("a-b.c", "a_b_c")]
        public void ToLocalIdentifier_AwkwardPapyrusNames_BecomeLegalLuaNames(string papyrus, string expected)
        {
            string local = NameMangling.ToLocalIdentifier(papyrus);

            Assert.Equal(expected, local);
            Assert.True(NameMangling.IsValidLuaIdentifier(local));
        }

        [Theory]
        [InlineData("plain")]
        [InlineData("with \"quotes\"")]
        [InlineData("back\\slash")]
        [InlineData("line\nbreak\rreturn\ttab")]
        [InlineData("nul\0inside")]
        [InlineData("bell\a\u007f")]
        [InlineData("digit after escape \u00019")]
        [InlineData("]]--[[ comment-looking")]
        [InlineData("")]
        public void ToLuaStringLiteral_AnyText_EvaluatesBackToTheOriginalString(string original)
        {
            string literal = NameMangling.ToLuaStringLiteral(original);

            string evaluated = new Script().DoString("return " + literal).String;

            Assert.Equal(original, evaluated);
        }
    }
}
