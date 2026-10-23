using AssetConverter;
using Xunit;

namespace InstallerTests
{
    public sealed class KeyValuesTextTests
    {
        [Fact]
        public void NestedObjectsAndQuotedValuesAreParsed()
        {
            KeyValuesNode root = KeyValuesText.Parse("\"libraryfolders\" { \"0\" { \"path\" \"D:\\\\Games\" \"apps\" { \"377160\" \"1\" } } }");

            KeyValuesNode library = root.Child("0")!;
            Assert.Equal(@"D:\Games", library.ValueOf("path"));
            Assert.Equal("1", library.Child("apps")!.ValueOf("377160"));
        }

        [Fact]
        public void CommentsAndLooseWhitespaceAreIgnored()
        {
            KeyValuesNode root = KeyValuesText.Parse("// header\r\n\"AppState\"\r\n{\r\n\t// note\r\n\t\"installdir\"\t\t\"Fallout 4 377160\" // trailing\r\n}\r\n");

            Assert.Equal("Fallout 4 377160", root.ValueOf("installdir"));
        }

        [Fact]
        public void UnquotedTokensAndEscapesAreParsed()
        {
            KeyValuesNode root = KeyValuesText.Parse("root { key \"a\\\"b\\tc\" }");

            Assert.Equal("a\"b\tc", root.ValueOf("key"));
        }

        [Fact]
        public void KeyLookupIgnoresCase()
        {
            Assert.Equal("v", KeyValuesText.Parse("\"r\" { \"Path\" \"v\" }").ValueOf("path"));
        }

        [Theory]
        [InlineData("")]
        [InlineData("\"root\"")]
        [InlineData("\"root\" { \"key\" \"value\"")]
        [InlineData("\"root\" { \"key\" }")]
        [InlineData("\"root\" { \"key\" \"unterminated }")]
        [InlineData("\"root\" { } extra")]
        [InlineData("{ \"key\" \"value\" }")]
        public void MalformedDocumentsAreRejected(string text)
        {
            Assert.Throws<KeyValuesFormatException>(() => KeyValuesText.Parse(text));
        }
    }
}
