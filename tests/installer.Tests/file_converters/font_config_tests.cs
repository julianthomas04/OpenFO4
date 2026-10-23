using AssetConverter;
using Xunit;

namespace InstallerTests
{
    public sealed class FontConfigTests
    {
        private const string Config =
            "fontlib \"Interface\\fonts_en.swf\"\r\n" +
            "map \"$Main\" = \"Some Family\" Normal\r\n" +
            "map \"$Main_Bold\" = \"Some Family Bold\" Bold\r\n" +
            "  map \"$Spaced\"   =   \"Spaced  Family\"   Normal  \r\n" +
            "validNameChars \"map \\\"$Fake\\\" = \\\"Nope\\\" Normal\"\r\n";

        [Theory]
        [InlineData("$Main", "Some Family")]
        [InlineData("$Main_Bold", "Some Family Bold")]
        [InlineData("$Spaced", "Spaced  Family")]
        public void FamilyOf_MappedClass_IsItsFamilyWithoutTheStyle(string fontClass, string family)
        {
            Assert.Equal(family, FontConfig.FamilyOf(Config, fontClass));
        }

        [Theory]
        [InlineData("$Unmapped")]
        [InlineData("$Fake")]
        [InlineData("$main")]
        public void FamilyOf_ClassWithoutAMapLine_ThrowsNamingTheClass(string fontClass)
        {
            var thrown = Assert.Throws<FontConfigException>(() => FontConfig.FamilyOf(Config, fontClass));

            Assert.Contains(fontClass, thrown.Message);
        }

        [Fact]
        public void FamilyOf_ClassMappedPerStyleToOneFamily_IsThatFamily()
        {
            const string Styles = "map \"$A\" = \"One\" Bold\nmap \"$A\" = \"One\" Normal\nmap \"$A\" = \"One\" Italic\n";

            Assert.Equal("One", FontConfig.FamilyOf(Styles, "$A"));
        }

        [Fact]
        public void FamilyOf_ClassMappedToSeveralFamilies_IsTheFamilyOfTheNormalLine()
        {
            const string Styles = "map \"$A\" = \"Bold Family\" Bold\nmap \"$A\" = \"Plain Family\" Normal\n";

            Assert.Equal("Plain Family", FontConfig.FamilyOf(Styles, "$A"));
        }

        [Fact]
        public void FamilyOf_ConflictingNormalLines_Throws()
        {
            var thrown = Assert.Throws<FontConfigException>(() =>
                FontConfig.FamilyOf("map \"$A\" = \"One\" Normal\nmap \"$A\" = \"Two\" Normal\n", "$A"));

            Assert.Contains("$A", thrown.Message);
        }

        [Fact]
        public void FamilyOf_SeveralLinesButNoNormalOne_Throws()
        {
            Assert.Throws<FontConfigException>(() =>
                FontConfig.FamilyOf("map \"$A\" = \"One\" Bold\nmap \"$A\" = \"Two\" Italic\n", "$A"));
        }

        [Fact]
        public void Parse_ListsEveryLibraryAndMapInOrderAndTheCharacterSets()
        {
            var config = FontConfig.Parse(
                "fontlib \"Interface\\fonts_en.swf\"\r\nfontlib \"Interface\\fonts_console.swf\"\r\n" +
                "map \"$Main\" = \"Some Family\" Normal\r\nvalidNameChars \"abc \"\r\nvalidBookChars \"xyz\"\r\n");

            Assert.Equal(["Interface\\fonts_en.swf", "Interface\\fonts_console.swf"], config.FontLibraries);
            Assert.Equal([new FontConfigMap("$Main", "Some Family", "Normal")], config.Maps);
            Assert.Equal("abc ", config.ValidNameChars);
            Assert.Equal("xyz", config.ValidBookChars);
        }

        [Fact]
        public void Parse_QuotedValues_UnescapeOnlyBackslashesAndQuotes()
        {
            var config = FontConfig.Parse("fontlib \"Interface\\fonts_en.swf\"\nvalidNameChars \"a\\\\b\\\"c\\d\"\nmap \"$\\\"Q\" = \"F\\\\G\" Normal\n");

            Assert.Equal(["Interface\\fonts_en.swf"], config.FontLibraries);
            Assert.Equal("a\\b\"c\\d", config.ValidNameChars);
            Assert.Equal([new FontConfigMap("$\"Q", "F\\G", "Normal")], config.Maps);
        }

        [Fact]
        public void Parse_UnescapedQuoteInsideAValue_Throws()
        {
            Assert.Throws<FontConfigException>(() => FontConfig.Parse("validNameChars \"a\"b\"\n"));
        }

        [Fact]
        public void Parse_WithoutCharacterSets_LeavesThemUnstated()
        {
            var config = FontConfig.Parse(Config.Replace("validNameChars", "fontlib", StringComparison.Ordinal));

            Assert.Null(config.ValidNameChars);
            Assert.Null(config.ValidBookChars);
        }

        [Theory]
        [InlineData("fontlib Interface\\fonts_en.swf")]
        [InlineData("include \"other.txt\"")]
        [InlineData("map \"$A\" = \"B\"")]
        public void Parse_LineOfNoKnownForm_ThrowsNamingTheLine(string line)
        {
            var thrown = Assert.Throws<FontConfigException>(() => FontConfig.Parse("map \"$A\" = \"B\" Normal\n" + line + "\n"));

            Assert.Contains("line 2", thrown.Message);
        }
    }
}
