using AssetConverter;
using AssetConverter.Pipeline;
using Dapper;
using Microsoft.Data.Sqlite;
using Mutagen.Bethesda;
using Mutagen.Bethesda.Fallout4;
using Mutagen.Bethesda.Plugins;
using Mutagen.Bethesda.Strings;
using Xunit;

namespace InstallerTests
{
    // Fallout4.esm stores names as ids into Fallout4_en.*STRINGS; the loader must resolve them from the extracted tables
    // and refuse to read a localized plugin without them.
    public sealed class EsmLocalizedStringsTests : IDisposable
    {
        const string ItemName = "Localized Stimpak";

        readonly TempDirectory _directory = new();
        readonly string _stringsFolder;
        readonly string _esmPath;

        public EsmLocalizedStringsTests()
        {
            _stringsFolder = Path.Combine(_directory.Path, "extracted", StringTables.FolderName);
            _esmPath = Path.Combine(_directory.Path, "Fallout4.esm");
            WriteLocalizedPlugin();
        }

        public void Dispose()
        {
            SqliteConnection.ClearAllPools();
            _directory.Dispose();
        }

        void WriteLocalizedPlugin()
        {
            var mod = new Fallout4Mod(ModKey.FromNameAndExtension("Fallout4.esm"), Fallout4Release.Fallout4);
            mod.ModHeader.Flags |= Fallout4ModHeader.HeaderFlag.Localized;
            var stimpak = mod.Ingestibles.AddNew("LocalizedStimpak");
            stimpak.Name = new TranslatedString(Language.English, ItemName);
            mod.WriteToBinary(_esmPath);
            Directory.CreateDirectory(Path.GetDirectoryName(_stringsFolder)!);
            Directory.Move(Path.Combine(_directory.Path, "Strings"), _stringsFolder);
        }

        string DatabasePath => Path.Combine(_directory.Path, "data.db");

        Task Load(string? stringsFolder)
        {
            File.Create(DatabasePath).Dispose();
            return new ESMLoader(DatabasePath) { Memory = MemoryBudget.Unlimited(), Strings = stringsFolder is null ? null : new StringTableSet(stringsFolder, "en") }
                .ReloadTablesAsync(_esmPath, TableReloadPlan.Create(["items"]), TestContext.Current.CancellationToken);
        }

        [Fact]
        public async Task Load_LocalizedPlugin_ResolvesNamesFromTheStringTables()
        {
            await Load(_stringsFolder);

            using var connection = new SqliteConnection(new SqliteConnectionStringBuilder { DataSource = DatabasePath, Pooling = false }.ToString());
            Assert.Equal(ItemName, connection.QuerySingle<string>("SELECT name FROM items"));
        }

        [Fact]
        public async Task Load_LocalizedPluginWithoutStringsFolder_Throws()
        {
            await Assert.ThrowsAsync<InvalidOperationException>(() => Load(null));
        }

        [Fact]
        public async Task Load_LocalizedPluginWithMissingTables_Throws()
        {
            string empty = Path.Combine(_directory.Path, "empty");
            Directory.CreateDirectory(empty);

            await Assert.ThrowsAsync<FileNotFoundException>(() => Load(empty));
        }
    }
}
