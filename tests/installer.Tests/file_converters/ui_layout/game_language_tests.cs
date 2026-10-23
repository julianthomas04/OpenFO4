using AssetConverter.UiLayout;
using Xunit;

namespace InstallerTests.UiLayout
{
    public sealed class GameLanguageTests : IDisposable
    {
        private readonly TempDirectory _root = new();

        public void Dispose() => _root.Dispose();

        private string Game => Path.Combine(_root.Path, "game");

        private string Settings => Path.Combine(_root.Path, "settings");

        private static void WriteIni(string directory, string name, string text)
        {
            Directory.CreateDirectory(directory);
            File.WriteAllText(Path.Combine(directory, name), text);
        }

        [Theory]
        [InlineData("[General]\nsLanguage=de\n", "de")]
        [InlineData("[general]\r\n  SLANGUAGE = ptbr  \r\n", "ptbr")]
        [InlineData("[General]\nsLanguage=en\nsLanguage=fr\n", "fr")]
        [InlineData("[General]\nsLanguage=it\n[Display]\nsLanguage=es\n", "es")]
        [InlineData("sLanguage=pl\n", "pl")]
        [InlineData("[Display]\nsLanguage:General=ru\n", "ru")]
        [InlineData("[General]\n; sLanguage=xx\nsLanguage=es\n", "es")]
        [InlineData("[General]\nsLanguage=ja ; Japanese\n", "ja")]
        [InlineData("\uFEFFsLanguage=cn\n", "cn")]
        public void FromText_IsTheLastStatementInAnySection(string ini, string expected)
        {
            Assert.Equal(expected, GameLanguage.FromText(ini));
        }

        [Theory]
        [InlineData("")]
        [InlineData("[General]\n;sLanguage=en\n")]
        [InlineData("[General]\nsLanguageX=en\n")]
        public void FromText_NoStatement_IsNone(string ini)
        {
            Assert.Null(GameLanguage.FromText(ini));
        }

        [Fact]
        public void Resolve_UserIniOverridesTheShippedDefault()
        {
            WriteIni(Game, "Fallout4_Default.ini", "[General]\nsLanguage=en\n");
            WriteIni(Settings, "Fallout4.ini", "[General]\nsLanguage=de\n");
            WriteIni(Settings, "Fallout4Prefs.ini", "[Display]\niSize W=1280\n");

            Assert.Equal("de", GameLanguage.Resolve(Game, Settings));
        }

        [Fact]
        public void Resolve_CustomIniWinsOverEveryOther()
        {
            WriteIni(Game, "Fallout4_Default.ini", "[General]\nsLanguage=en\n");
            WriteIni(Settings, "Fallout4.ini", "[General]\nsLanguage=de\n");
            WriteIni(Settings, "Fallout4Custom.ini", "[General]\nsLanguage=fr\n");

            Assert.Equal("fr", GameLanguage.Resolve(Game, Settings));
        }

        [Fact]
        public void Resolve_NoIniStatesTheLanguage_Throws()
        {
            WriteIni(Game, "Fallout4_Default.ini", "[Display]\n");

            Assert.Throws<GameLanguageException>(() => GameLanguage.Resolve(Game, Settings));
        }

        [Fact]
        public void Resolve_UppercaseLanguage_IsUsedInLowercase()
        {
            WriteIni(Game, "Fallout4_Default.ini", "[General]\nsLanguage=EN\n");

            Assert.Equal("en", GameLanguage.Resolve(Game, Settings));
        }

        [Theory]
        [InlineData("../en")]
        [InlineData("en us")]
        [InlineData("")]
        public void Resolve_LanguageThatIsNotLetters_Throws(string language)
        {
            WriteIni(Game, "Fallout4_Default.ini", $"[General]\nsLanguage={language}\n");

            Assert.Throws<GameLanguageException>(() => GameLanguage.Resolve(Game, Settings));
        }
    }
}
