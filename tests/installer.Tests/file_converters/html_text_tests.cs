using AssetConverter.UiLayout;
using Xunit;

namespace InstallerTests
{
    public sealed class HtmlTextTests
    {
        private const string Field =
            "<TEXTFORMAT LEADING='2'><P ALIGN=\"LEFT\"><FONT FACE=\"$Main\" SIZE=\"20\">Hi <B>there</B>&amp;</FONT></P><P>x<BR>y</P></TEXTFORMAT>";

        [Fact]
        public void Parse_Runs_CarryTheFormatOfEveryEnclosingTag()
        {
            var runs = HtmlText.Parse(Field).Runs;

            Assert.Equal(["Hi ", "there", "&", "\nx\ny"], runs.Select(r => r.Text));
            Assert.Equal("2", runs[0].Format["TEXTFORMAT.LEADING"]);
            Assert.Equal("$Main", runs[0].Format["FONT.FACE"]);
            Assert.False(runs[0].Format.ContainsKey("B"));
            Assert.Equal(HtmlText.StyleOn, runs[1].Format["B"]);
            Assert.False(runs[2].Format.ContainsKey("B"));
            Assert.False(runs[3].Format.ContainsKey("FONT.FACE"));
        }

        [Fact]
        public void Parse_Attributes_AreTheFirstValueOfEachTagAttributeInEitherQuoteStyle()
        {
            var attributes = HtmlText.Parse(Field + "<P ALIGN=\"RIGHT\">z</P>").Attributes;

            Assert.Equal("2", attributes["TEXTFORMAT.LEADING"]);
            Assert.Equal("LEFT", attributes["P.ALIGN"]);
            Assert.Equal("20", attributes["FONT.SIZE"]);
        }

        [Fact]
        public void Parse_UnquotedAttributeValues_AreKept()
        {
            var attributes = HtmlText.Parse("<FONT SIZE=20 COLOR=#ffffff>x</FONT>").Attributes;

            Assert.Equal("20", attributes["FONT.SIZE"]);
            Assert.Equal("#ffffff", attributes["FONT.COLOR"]);
        }

        [Fact]
        public void Parse_GreaterThanInsideAQuotedValue_DoesNotEndTheTag()
        {
            var runs = HtmlText.Parse("<FONT FACE=\"a>b\">x</FONT>").Runs;

            Assert.Equal("x", Assert.Single(runs).Text);
            Assert.Equal("a>b", runs[0].Format["FONT.FACE"]);
        }

        [Fact]
        public void Parse_SelfClosingTagWithAnUnquotedValue_IsNotLeftOpen()
        {
            var runs = HtmlText.Parse("<FONT SIZE=20/>x").Runs;

            Assert.False(Assert.Single(runs).Format.ContainsKey("FONT.SIZE"));
        }
    }
}
