using System.Text;
using System.Text.Json;
using AssetConverter;
using AssetConverter.UiLayout;
using Xunit;

namespace InstallerTests.UiLayout
{
    public sealed class TranslateTableTests
    {
        private static byte[] Utf16(string text) => [.. Encoding.Unicode.GetPreamble(), .. Encoding.Unicode.GetBytes(text)];

        private static TranslateTable Parse(string text) => TranslateTable.Parse(Utf16(text), "Translate_xx.txt");

        [Fact]
        public void Parse_EachLineIsAKeyATabAndItsText()
        {
            var table = Parse("$Ok\tOkay\r\n$Cancel\tGo back\r\n");

            Assert.Equal(new Dictionary<string, string> { ["$Cancel"] = "Go back", ["$Ok"] = "Okay" }, table.Entries);
            Assert.Empty(table.Unresolved);
        }

        [Fact]
        public void Parse_TabsAroundTheText_AreNotPartOfIt()
        {
            Assert.Equal("Two words", Parse("$A\t\t\tTwo words\t\r\n").Entries["$A"]);
        }

        [Fact]
        public void Parse_TextKeepsInnerTabsSpacesAndOtherCharacters()
        {
            Assert.Equal(" x\ty $B ", Parse("$A\t x\ty $B \r\n").Entries["$A"]);
        }

        [Fact]
        public void Parse_KeysAreCaseSensitive()
        {
            var table = Parse("$ok\tlower\n$OK\tupper\n");

            Assert.Equal("lower", table.Entries["$ok"]);
            Assert.Equal("upper", table.Entries["$OK"]);
        }

        [Fact]
        public void Parse_KeyStatedTwiceWithTheSameText_IsOneEntry()
        {
            Assert.Equal("Same", Parse("$A\tSame\n$A\tSame\n").Entries["$A"]);
        }

        [Fact]
        public void Parse_KeyStatedWithDifferentTexts_IsUnresolved()
        {
            var table = Parse("$A\tOne\n$A\tTwo\n$A\tOne\n$B\tFine\n");

            Assert.False(table.Entries.ContainsKey("$A"));
            Assert.Equal("stated with 2 different texts", table.Unresolved["$A"]);
            Assert.Equal("Fine", table.Entries["$B"]);
        }

        [Fact]
        public void Parse_BlankLines_AreSkipped()
        {
            Assert.Single(Parse("\r\n$A\tx\r\n   \r\n").Entries);
        }

        [Theory]
        [InlineData("A\tno marker")]
        [InlineData("$A without a tab")]
        [InlineData("\t$A")]
        public void Parse_LineOfAnotherForm_ThrowsNamingTheLine(string line)
        {
            var thrown = Assert.Throws<TranslateTableException>(() => Parse("$Ok\tOkay\n" + line + "\n"));

            Assert.Contains("line 2", thrown.Message);
        }

        [Fact]
        public void ToJson_WritesTheHeaderLanguageEntriesAndUnresolvedKeys()
        {
            var stamp = UiDocumentHeader.Stamp("xx", SourceFingerprint.Of(@"Interface\Translate_xx.txt", [1, 2, 3]));

            using var json = JsonDocument.Parse(Parse("$B\tb\n$A\ta\n$C\t1\n$C\t2\n").ToJson("xx", stamp));

            JsonElement root = json.RootElement;
            Assert.Equal("translate", root.GetProperty("kind").GetString());
            Assert.Equal("xx", root.GetProperty("language").GetString());
            Assert.Equal(@"Interface\Translate_xx.txt", root.GetProperty("conversion").GetProperty("sources")[0].GetProperty("entry").GetString());
            Assert.Equal(["$A", "$B"], root.GetProperty("entries").EnumerateObject().Select(p => p.Name));
            Assert.Equal("stated with 2 different texts", root.GetProperty("unresolved").GetProperty("$C").GetString());
        }
    }
}
