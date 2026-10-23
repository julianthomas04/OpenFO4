using AssetConverter;
using InstallerTests.Archives;
using Xunit;

namespace InstallerTests
{
    // The staged tables are a current extraction of the installed archive, or they are extracted again.
    public sealed class StringTablesTests : IDisposable
    {
        private readonly TempDirectory _root = new();

        public void Dispose() => _root.Dispose();

        private string Game => Path.Combine(_root.Path, "game");

        private string Staging => Path.Combine(_root.Path, "staging");

        private string Table(string extension) => Path.Combine(StringTables.Directory(Staging), "Fallout4_en" + extension);

        private void WriteArchive(byte[] stringsContent)
        {
            Directory.CreateDirectory(Path.Combine(Game, "Data"));
            File.WriteAllBytes(Path.Combine(Game, "Data", "Fallout4 - Interface.ba2"), Ba2Builder.Gnrl(
            [
                new Ba2File(@"STRINGS\Fallout4_en.STRINGS", stringsContent, Compress: false),
                new Ba2File(@"STRINGS\Fallout4_en.DLSTRINGS", [2], Compress: false),
                new Ba2File(@"STRINGS\Fallout4_en.ILSTRINGS", [3], Compress: false),
                new Ba2File(@"STRINGS\Fallout4_de.STRINGS", [9], Compress: false),
            ]));
        }

        [Fact]
        public void Extract_WritesTheLanguagesTablesAndOnlyThose()
        {
            WriteArchive([1]);

            var tables = StringTables.Extract(Game, Staging, "en");

            Assert.Equal(StringTables.Directory(Staging), tables.Folder);
            Assert.Equal([1], File.ReadAllBytes(Table(".STRINGS")));
            Assert.Equal([2], File.ReadAllBytes(Table(".DLSTRINGS")));
            Assert.False(File.Exists(Path.Combine(tables.Folder, "Fallout4_de.STRINGS")));
        }

        [Fact]
        public void Extract_TablesOfTheCurrentArchive_AreLeftAlone()
        {
            WriteArchive([1]);
            StringTables.Extract(Game, Staging, "en");
            DateTime written = File.GetLastWriteTimeUtc(Table(".STRINGS"));
            Thread.Sleep(20);

            StringTables.Extract(Game, Staging, "en");

            Assert.Equal(written, File.GetLastWriteTimeUtc(Table(".STRINGS")));
        }

        [Fact]
        public void Extract_ArchiveThatChanged_RefreshesTheStagedTables()
        {
            WriteArchive([1]);
            StringTables.Extract(Game, Staging, "en");

            WriteArchive([7, 7]);
            StringTables.Extract(Game, Staging, "en");

            Assert.Equal([7, 7], File.ReadAllBytes(Table(".STRINGS")));
        }

        [Fact]
        public void Extract_TableDeletedFromTheStagingFolder_IsExtractedAgain()
        {
            WriteArchive([1]);
            StringTables.Extract(Game, Staging, "en");
            File.Delete(Table(".ILSTRINGS"));

            StringTables.Extract(Game, Staging, "en");

            Assert.Equal([3], File.ReadAllBytes(Table(".ILSTRINGS")));
        }

        [Fact]
        public void Extract_LanguageTheArchiveLacks_Throws()
        {
            WriteArchive([1]);

            Assert.Throws<FileNotFoundException>(() => StringTables.Extract(Game, Staging, "fr"));
        }
    }
}
