using AssetConverter;
using InstallerTests.Archives;
using Xunit;

namespace InstallerTests
{
    /// <summary>The stage logs to the console, which other tests redirect, so it runs serially with them.</summary>
    [Collection("Console redirection")]
    public sealed class LodWaterStageTests
    {
        const string MeshesArchive = "Fallout4 - Meshes.ba2";
        const string MeshesExtraArchive = "Fallout4 - MeshesExtra.ba2";
        const string WorldFormKey = "000001:Fallout4.esm";

        static Ba2File Tile(string world, int x, int y, double height) =>
            new($"Meshes\\Terrain\\{world}\\{world}.4.{x}.{y}.BTR", new BtrBuilder().Shape(new FixtureQuad(0, 0, height)).Build(), Compress: false);

        static Ba2File Unrelated() => new("meshes\\other\\thing.nif", [1, 2, 3], Compress: false);

        static void WriteDataDirectory(ArchiveTempDir dir, IReadOnlyList<Ba2File> meshes, IReadOnlyList<Ba2File> meshesExtra)
        {
            dir.Write(MeshesArchive, Ba2Builder.Gnrl(meshes));
            dir.Write(MeshesExtraArchive, Ba2Builder.Gnrl(meshesExtra));
        }

        [Fact]
        public void Scan_ReadsBothArchivesWithoutADatabase()
        {
            using var dir = new ArchiveTempDir();
            WriteDataDirectory(dir, [Tile("World", 0, 0, 450), Unrelated()], [Tile("World", 8, 0, 460)]);

            var heights = LodWaterStage.Scan(dir.Path, TestContext.Current.CancellationToken);

            Assert.Equal(2, heights.Report.FilesWithWater);
            Assert.Equal(450.0, heights.Heights[new LodWaterCell("World", 0, 0)]);
            Assert.Equal(460.0, heights.Heights[new LodWaterCell("World", 8, 0)]);
        }

        [Fact]
        public void Scan_MissingArchive_Throws()
        {
            using var dir = new ArchiveTempDir();
            dir.Write(MeshesArchive, Ba2Builder.Gnrl([Unrelated()]));

            var error = Assert.Throws<FileNotFoundException>(() => LodWaterStage.Scan(dir.Path, TestContext.Current.CancellationToken));

            Assert.EndsWith(MeshesExtraArchive, error.FileName);
        }

        [Fact]
        public void Scan_CancelledBeforeItReadsAnArchive_ThrowsAndLeavesTheArchivesUnlocked()
        {
            using var dir = new ArchiveTempDir();
            WriteDataDirectory(dir, [Tile("World", 0, 0, 450)], [Tile("World", 8, 0, 460)]);
            using var cancelled = new CancellationTokenSource();
            cancelled.Cancel();

            Assert.Throws<OperationCanceledException>(() => LodWaterStage.Scan(dir.Path, cancelled.Token));

            File.Delete(Path.Combine(dir.Path, MeshesArchive));
        }

        [Fact]
        public void Scan_MissingDataDirectory_Throws()
        {
            using var dir = new ArchiveTempDir();

            Assert.Throws<DirectoryNotFoundException>(() => LodWaterStage.Scan(Path.Combine(dir.Path, "absent"), TestContext.Current.CancellationToken));
        }

        [Fact]
        public async Task Run_ScansTheArchivesAndWritesTheTable()
        {
            using var dir = new ArchiveTempDir();
            WriteDataDirectory(dir, [Tile("World", 0, 0, 450)], [Unrelated()]);
            using var db = await SchemaDatabase.CreateAsync();
            db.Execute($"INSERT INTO worldspaces (form_key, editor_id) VALUES ('{WorldFormKey}', 'World')");

            var report = LodWaterStage.Run(dir.Path, db.DatabasePath);

            Assert.Equal(1, report.FilesWithWater);
            Assert.Equal([(WorldFormKey, 0, 0, 450.0)],
                db.Query<(string Worldspace, int CellX, int CellY, double Height)>("SELECT worldspace, cell_x, cell_y, height FROM lod_water_quads"));
        }

        [Fact]
        public void Run_MissingDatabase_FailsBeforeScanning()
        {
            using var dir = new ArchiveTempDir();
            string missingDatabase = Path.Combine(dir.Path, "data.db");

            var error = Assert.Throws<FileNotFoundException>(() => LodWaterStage.Run(dir.Path, missingDatabase));

            Assert.Equal(missingDatabase, error.FileName);
        }

        [Fact]
        public void WriteTable_MissingDatabase_ThrowsWithoutCreatingIt()
        {
            using var dir = new ArchiveTempDir();
            string missingDatabase = Path.Combine(dir.Path, "data.db");
            var heights = new LodWaterHeights(new Dictionary<LodWaterCell, double>(), new LodWaterReport(0, 0, 0, 0, 0, new Dictionary<string, int>()));

            Assert.Throws<FileNotFoundException>(() => LodWaterStage.WriteTable(missingDatabase, heights));
            Assert.False(File.Exists(missingDatabase));
        }
    }
}
