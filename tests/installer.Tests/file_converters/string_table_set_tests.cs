using AssetConverter;
using Mutagen.Bethesda.Strings;
using Xunit;

namespace InstallerTests
{
    public sealed class StringTableSetTests
    {
        [Theory]
        [InlineData("en", Language.English)]
        [InlineData("de", Language.German)]
        [InlineData("fr", Language.French)]
        [InlineData("ja", Language.Japanese)]
        public void PluginLanguage_IsTheGameLanguageCodeAsThePluginReaderNamesIt(string code, Language expected)
        {
            Assert.Equal(expected, new StringTableSet("STRINGS", code).PluginLanguage);
        }

        [Fact]
        public void PluginLanguage_OfAnUnknownCode_Throws()
        {
            Assert.Throws<InvalidOperationException>(() => new StringTableSet("STRINGS", "xx").PluginLanguage);
        }

        [Fact]
        public void Require_MissingTablesOfTheLanguage_ThrowsNamingThem()
        {
            using var folder = new TempDirectory();
            File.WriteAllBytes(Path.Combine(folder.Path, "Fallout4_en.STRINGS"), []);
            File.WriteAllBytes(Path.Combine(folder.Path, "Fallout4_en.DLSTRINGS"), []);
            File.WriteAllBytes(Path.Combine(folder.Path, "Fallout4_en.ILSTRINGS"), []);

            StringTables.Require(new StringTableSet(folder.Path, "en"), "the test");
            var thrown = Assert.Throws<FileNotFoundException>(() => StringTables.Require(new StringTableSet(folder.Path, "de"), "the test"));

            Assert.Contains("Fallout4_de.STRINGS", thrown.Message);
        }
    }
}
