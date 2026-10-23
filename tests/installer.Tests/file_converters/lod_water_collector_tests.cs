using AssetConverter;
using Xunit;

namespace InstallerTests
{
    public sealed class LodWaterCollectorTests
    {
        static (string Path, Func<byte[]> ReadBytes) Entry(string path, BtrBuilder builder)
        {
            var bytes = builder.Build();
            return (path, () => bytes);
        }

        static string L4(string world, int x, int y) => $"Meshes\\Terrain\\{world}\\{world}.4.{x}.{y}.BTR";

        [Theory]
        [InlineData("Meshes\\Terrain\\Commonwealth\\Commonwealth.4.-12.68.BTR", "Commonwealth", 4, -12, 68)]
        [InlineData("meshes/terrain/NukaWorld/NukaWorld.8.16.-16.btr", "NukaWorld", 8, 16, -16)]
        public void TryParse_TerrainLodPath_ReturnsParts(string path, string world, int level, int x, int y)
        {
            Assert.True(BtrFileName.TryParse(path, out var name));
            Assert.Equal(new BtrFileName(world, level, x, y), name);
        }

        [Theory]
        [InlineData("Meshes\\Terrain\\Commonwealth\\Commonwealth.4.0.btr")]
        [InlineData("Meshes\\Terrain\\Commonwealth\\Other.4.0.0.BTR")]
        [InlineData("Meshes\\Landscape\\Commonwealth\\Commonwealth.4.0.0.BTR")]
        public void TryParse_OtherPaths_ReturnsFalse(string path)
        {
            Assert.False(BtrFileName.TryParse(path, out _));
        }

        [Fact]
        public void Collect_SkipsOtherLevelsAndReportsCounts()
        {
            var result = LodWaterCollector.Collect([
                Entry(L4("World", 0, 0), new BtrBuilder().Shape(new FixtureQuad(0, 0, 450))),
                Entry(L4("World", 4, 0), new BtrBuilder { IncludeWaterNode = false }),
                Entry("Meshes\\Terrain\\World\\World.8.0.0.BTR", new BtrBuilder().Shape(new FixtureQuad(0, 0, 999))),
            ]);

            Assert.Equal(450, Assert.Single(result.Heights).Value);
            Assert.Equal(2, result.Report.FilesRead);
            Assert.Equal(1, result.Report.FilesWithWater);
            Assert.Equal(1, result.Report.QuadsPerWorld["World"]);
        }

        [Fact]
        public void Collect_SameCellAndHeightTwiceInOneFile_IsOneRowCountedAsWithinFile()
        {
            var result = LodWaterCollector.Collect([
                Entry(L4("World", 0, 0), new BtrBuilder().Shape(new FixtureQuad(1, 1, 450)).Shape(new FixtureQuad(1, 1, 450))),
            ]);

            Assert.Single(result.Heights);
            Assert.Equal(1, result.Report.DuplicatesWithinFile);
            Assert.Equal(0, result.Report.DuplicatesAcrossFiles);
        }

        [Fact]
        public void Collect_OverlappingFiles_CountsAcrossFiles()
        {
            var result = LodWaterCollector.Collect([
                Entry(L4("World", 0, 0), new BtrBuilder().Shape(new FixtureQuad(4, 0, 450))),
                Entry(L4("World", 4, 0), new BtrBuilder().Shape(new FixtureQuad(0, 0, 450))),
            ]);

            Assert.Single(result.Heights);
            Assert.Equal(1, result.Report.DuplicatesAcrossFiles);
        }

        [Fact]
        public void Collect_SameCellDifferentHeights_Throws()
        {
            var e = Assert.Throws<LodWaterConflictException>(() => LodWaterCollector.Collect([
                Entry(L4("World", 0, 0), new BtrBuilder().Shape(new FixtureQuad(0, 0, 450)).Shape(new FixtureQuad(0, 0, 458))),
            ]));

            Assert.Contains("(0,0)", e.Message);
        }

        [Fact]
        public async Task WriteTable_StoresWorldspaceFormKeyAndReplacesPreviousRows()
        {
            using var db = await SchemaDatabase.CreateAsync();
            db.Execute("INSERT INTO worldspaces (form_key, editor_id) VALUES ('000001:Fallout4.esm', 'World')");
            var entries = new[] { Entry(L4("world", 0, 0), new BtrBuilder().Shape(new FixtureQuad(0, 0, 450), new FixtureQuad(1, 0, 458))) };

            LodWaterStage.WriteTable(db.DatabasePath, LodWaterCollector.Collect(entries));
            LodWaterStage.WriteTable(db.DatabasePath, LodWaterCollector.Collect(entries));

            var rows = db.Query<(string Worldspace, int CellX, int CellY, double Height)>(
                "SELECT worldspace, cell_x, cell_y, height FROM lod_water_quads ORDER BY cell_x");
            Assert.Equal([("000001:Fallout4.esm", 0, 0, 450.0), ("000001:Fallout4.esm", 1, 0, 458.0)], rows);
        }

        [Fact]
        public async Task WriteTable_FolderWithoutWorldspace_Throws()
        {
            using var db = await SchemaDatabase.CreateAsync();
            var entries = new[] { Entry(L4("Ghost", 0, 0), new BtrBuilder().Shape(new FixtureQuad(0, 0, 450))) };

            Assert.Throws<UnknownLodWorldspaceException>(() => LodWaterStage.WriteTable(db.DatabasePath, LodWaterCollector.Collect(entries)));
        }
    }
}
