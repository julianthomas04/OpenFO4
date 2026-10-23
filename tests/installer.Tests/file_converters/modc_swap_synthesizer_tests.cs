using System.Text;
using AssetConverter;
using Dapper;
using InstallerTests.Support;
using Microsoft.Data.Sqlite;
using Xunit;

namespace InstallerTests
{
    // ModcSwapSynthesizer: bases with a MODC color-remapping float get a synthetic "MODC:<base>" material swap whose entries
    // re-row the model's palette materials, and the palette variants those entries name are baked into the textures output.
    // The census is process-wide, so the staged materials are built once per class and the class shares the static-state collection.
    [Collection(StaticStateCollections.NifConverterState)]
    public sealed class ModcSwapSynthesizerTests : IClassFixture<ModcSwapSynthesizerTests.StagedMaterials>, IDisposable
    {
        const string PaletteDiffuse = "modc_test/paint_d.dds";
        const string MissingDiffuse = "modc_test/missing_d.dds";
        const string Lut = "modc_test/paint_lut.dds";
        const string PaintMaterial = "materials/modc_test/paint.bgsm";
        const string BrokenMaterial = "materials/modc_test/broken.bgsm";
        const string PlainMaterial = "materials/modc_test/plain.bgsm";

        public sealed class StagedMaterials : IDisposable
        {
            readonly TempDirectory _directory = new();

            public StagedMaterials()
            {
                Root = Path.Combine(_directory.Path, "staging");
                WriteMaterial("paint", PaletteDiffuse, palette: true);
                WriteMaterial("broken", MissingDiffuse, palette: true);
                WriteMaterial("plain", PaletteDiffuse, palette: false);
                var lutPath = Path.Combine(Root, "textures", Lut);
                Directory.CreateDirectory(Path.GetDirectoryName(lutPath)!);
                File.WriteAllBytes(lutPath, SyntheticDds.Bgra32(4, 2, Bgra(
                    (0, 0, 255, 255), (0, 255, 0, 255), (255, 0, 0, 255), (255, 255, 255, 255),
                    (0, 0, 100, 255), (0, 100, 0, 255), (100, 0, 0, 255), (100, 100, 100, 255))));
                File.WriteAllBytes(Path.Combine(Root, "textures", PaletteDiffuse), SyntheticDds.Bgra32(2, 2, Bgra(
                    (0, 0, 0, 255), (64, 64, 64, 255), (128, 128, 128, 255), (255, 255, 255, 255))));
                NifConverter.PreloadTextureIndex([$"textures\\{PaletteDiffuse}", $"textures\\{Lut}"]);
                MaterialCensus.Build(Root);
            }

            public string Root { get; }

            void WriteMaterial(string name, string diffuse, bool palette)
            {
                string path = Path.Combine(Root, "materials", "modc_test", name + ".bgsm");
                Directory.CreateDirectory(Path.GetDirectoryName(path)!);
                File.WriteAllBytes(path, new BgsmFields
                {
                    Diffuse = "textures\\" + diffuse.Replace('/', '\\'),
                    Greyscale = palette ? "textures\\" + Lut.Replace('/', '\\') : "",
                    Base = new MaterialBaseFields { GrayscaleToPalette = palette },
                    GrayscaleToPaletteScale = 0f,
                }.Build(2));
            }

            public void Dispose()
            {
                NifConverter.PreloadTextureIndex([]);
                _directory.Dispose();
            }
        }

        readonly StagedMaterials _staged;
        readonly TempDirectory _output = new();
        readonly SchemaDatabase _db = SchemaDatabase.CreateAsync().GetAwaiter().GetResult();
        readonly string _meshesOut;
        readonly string _texturesOut;

        public ModcSwapSynthesizerTests(StagedMaterials staged)
        {
            _staged = staged;
            _meshesOut = Path.Combine(_output.Path, "meshes");
            _texturesOut = Path.Combine(_output.Path, "textures");
            Directory.CreateDirectory(_meshesOut);
            Directory.CreateDirectory(_texturesOut);
        }

        public void Dispose()
        {
            _db.Dispose();
            _output.Dispose();
        }

        static byte[] Bgra(params (byte B, byte G, byte R, byte A)[] pixels) =>
            pixels.SelectMany(p => new[] { p.B, p.G, p.R, p.A }).ToArray();

        static byte[] Glb(params string[] materialSources)
        {
            string materials = string.Join(",", materialSources.Select(source => $"{{\"name\":\"mat|src:{source}\"}}"));
            string text = $"{{\"asset\":{{\"version\":\"2.0\"}},\"materials\":[{materials}]}}";
            var json = Encoding.UTF8.GetBytes(text.PadRight((text.Length + 3) / 4 * 4));
            using var stream = new MemoryStream();
            using var writer = new BinaryWriter(stream);
            writer.Write(0x46546C67u);
            writer.Write(2u);
            writer.Write((uint)(12 + 8 + json.Length));
            writer.Write((uint)json.Length);
            writer.Write(0x4E4F534Au);
            writer.Write(json);
            return stream.ToArray();
        }

        void AddBaseSwap(string formKey, float? modc, string? swapFormKey = null) =>
            _db.Execute("INSERT INTO base_material_swaps (form_key, material_swap, color_remap_index) VALUES (@F, @S, @C)", new { F = formKey, S = swapFormKey, C = modc });

        void AddBase(string formKey, string model, float modc, string? swapFormKey = null, string? lodModel = null)
        {
            _db.Execute("INSERT INTO statics (form_key, model_path, lod_model_path) VALUES (@F, @M, @L)", new { F = formKey, M = model, L = lodModel });
            AddBaseSwap(formKey, modc, swapFormKey);
        }

        void WriteModel(string model, byte[] glb)
        {
            string path = Path.Combine(_meshesOut, Path.ChangeExtension(model, ".glb"));
            Directory.CreateDirectory(Path.GetDirectoryName(path)!);
            File.WriteAllBytes(path, glb);
        }

        Task<ModcSynthesisSummary> RunAsync(CancellationToken ct) =>
            ModcSwapSynthesizer.RunAsync(_db.Connection, null, _meshesOut, _staged.Root, _texturesOut, ct);

        static string VariantFileName(string diffuse, float row) => MaterialCensus.VariantName(diffuse, Lut, row).Replace('/', Path.DirectorySeparatorChar);

        IEnumerable<string> BakedFiles() =>
            Directory.GetFiles(_texturesOut, "*", SearchOption.AllDirectories).Select(f => Path.GetRelativePath(_texturesOut, f));

        [Fact]
        public async Task RunAsync_ManyBases_InsertsSwapsAndEntriesInBaseOrderAndBakesEveryVariantLikeASerialBake()
        {
            float[] rows = [0.9f, 0.1f, 0.5f, 0.3f, 0.7f, 0.2f, 0.8f, 0.4f, 0.6f, 0.15f, 0.55f, 0.35f];
            for (int i = 0; i < rows.Length; i++)
            {
                WriteModel($"modc_test/model{i}.nif", Glb(PaintMaterial));
                AddBase($"0000{i:X2}:Fixture.esm", $"modc_test\\model{i}.nif", rows[i]);
            }

            await RunAsync(TestContext.Current.CancellationToken);

            var expectedSwaps = Enumerable.Range(0, rows.Length).Select(i => $"MODC:0000{i:X2}:Fixture.esm").ToList();
            Assert.Equal(expectedSwaps, _db.Query<string>("SELECT form_key FROM material_swaps ORDER BY rowid"));
            Assert.Equal(expectedSwaps, _db.Query<string>("SELECT swap_form_key FROM material_swap_entries ORDER BY rowid"));
            Assert.Equal(expectedSwaps, _db.Query<string>("SELECT material_swap FROM base_material_swaps ORDER BY rowid"));
            Assert.Equal(
                rows.Select(row => MaterialCensus.VariantName(PaletteDiffuse, Lut, row)),
                _db.Query<string>("SELECT diffuse FROM material_swap_entries ORDER BY rowid"));

            string serialOut = Path.Combine(_output.Path, "serial");
            foreach (float row in rows)
                Assert.True(PaletteBaker.BakeCombo(_staged.Root, serialOut, PaletteDiffuse, Lut, row));
            var expectedFiles = rows.Select(row => VariantFileName(PaletteDiffuse, row)).Order().ToList();
            Assert.Equal(expectedFiles, BakedFiles().Order());
            Assert.All(expectedFiles, file => Assert.Equal(
                File.ReadAllBytes(Path.Combine(serialOut, file)),
                File.ReadAllBytes(Path.Combine(_texturesOut, file))));
        }

        [Fact]
        public async Task RunAsync_BasesSharingOneModelAndRow_ShareOneBakedVariant()
        {
            WriteModel("modc_test/shared.nif", Glb(PaintMaterial));
            AddBase("000001:Fixture.esm", "modc_test\\shared.nif", 0.5f);
            AddBase("000002:Fixture.esm", "modc_test\\shared.nif", 0.5f);

            await RunAsync(TestContext.Current.CancellationToken);

            Assert.Equal(["MODC:000001:Fixture.esm", "MODC:000002:Fixture.esm"], _db.Query<string>("SELECT form_key FROM material_swaps ORDER BY rowid"));
            Assert.Equal([VariantFileName(PaletteDiffuse, 0.5f)], BakedFiles());
        }

        [Fact]
        public async Task RunAsync_RowOfTheInPlaceWinner_PointsBackAtTheDefaultTextureAndBakesNothing()
        {
            WriteModel("modc_test/winner.nif", Glb(PaintMaterial));
            AddBase("000001:Fixture.esm", "modc_test\\winner.nif", 0f);

            await RunAsync(TestContext.Current.CancellationToken);

            Assert.Equal([PaletteDiffuse], _db.Query<string>("SELECT diffuse FROM material_swap_entries"));
            Assert.Empty(BakedFiles());
        }

        [Fact]
        public async Task RunAsync_BaseWithoutModelOrPaletteMaterial_GetsNoSwapAndTheOthersAreUnaffected()
        {
            WriteModel("modc_test/palette.nif", Glb(PaintMaterial, PlainMaterial));
            WriteModel("modc_test/plain.nif", Glb(PlainMaterial));
            AddBaseSwap("000001:Fixture.esm", 0.5f);
            AddBase("000002:Fixture.esm", "modc_test\\plain.nif", 0.5f);
            AddBase("000003:Fixture.esm", "modc_test\\palette.nif", 0.5f);

            await RunAsync(TestContext.Current.CancellationToken);

            Assert.Equal(["MODC:000003:Fixture.esm"], _db.Query<string>("SELECT form_key FROM material_swaps"));
            Assert.Equal(["MODC:000003:Fixture.esm"], _db.Query<string>("SELECT swap_form_key FROM material_swap_entries"));
        }

        [Fact]
        public async Task RunAsync_ModelWithALodModel_ReRowsThePaletteMaterialsOfBoth()
        {
            WriteModel("modc_test/full.nif", Glb(PaintMaterial));
            WriteModel("lod/modc_test/full_lod.nif", Glb(BrokenMaterial));
            AddBase("000001:Fixture.esm", "modc_test\\full.nif", 0.5f, lodModel: "lod\\modc_test\\full_lod.nif");

            await RunAsync(TestContext.Current.CancellationToken);

            Assert.Equal(
                [PaintMaterial, BrokenMaterial],
                _db.Query<string>("SELECT original_material FROM material_swap_entries ORDER BY rowid"));
        }

        [Fact]
        public async Task RunAsync_ModsAndModcBase_KeepsTheRealSwapEntriesAndRecolorsTheRest()
        {
            _db.Execute("INSERT INTO material_swaps (form_key, editor_id) VALUES ('000010:Fixture.esm', 'RealSwap')");
            _db.Execute(
                "INSERT INTO material_swap_entries (swap_form_key, original_material, replacement_material, diffuse) VALUES ('000010:Fixture.esm', @O, 'materials/other.bgsm', 'other/real_d.dds')",
                new { O = PaintMaterial });
            WriteModel("modc_test/both.nif", Glb(PaintMaterial, BrokenMaterial));
            AddBase("000001:Fixture.esm", "modc_test\\both.nif", 0.5f, swapFormKey: "000010:Fixture.esm");

            await RunAsync(TestContext.Current.CancellationToken);

            var entries = _db.Query<(string Original, string Diffuse)>(
                "SELECT original_material AS Original, diffuse AS Diffuse FROM material_swap_entries WHERE swap_form_key = 'MODC:000001:Fixture.esm' ORDER BY rowid");
            Assert.Equal(
                [(PaintMaterial, "other/real_d.dds"), (BrokenMaterial, MaterialCensus.VariantName(MissingDiffuse, Lut, 0.5f))],
                entries);
        }

        [Fact]
        public async Task RunAsync_UnreadableModel_IsTreatedAsHavingNoPaletteMaterialsWhileOtherModelsAreSynthesized()
        {
            WriteModel("modc_test/garbage.nif", [1, 2, 3, 4, 5, 6, 7, 8]);
            WriteModel("modc_test/good.nif", Glb(PaintMaterial));
            AddBase("000001:Fixture.esm", "modc_test\\garbage.nif", 0.5f);
            AddBase("000002:Fixture.esm", "modc_test\\good.nif", 0.5f);

            await RunAsync(TestContext.Current.CancellationToken);

            Assert.Equal(["MODC:000002:Fixture.esm"], _db.Query<string>("SELECT form_key FROM material_swaps"));
        }

        [Fact]
        public async Task RunAsync_MissingOrUnreadableModelsAndFailedBakes_AreCountedInTheSummary()
        {
            WriteModel("modc_test/garbage.nif", [1, 2, 3, 4, 5, 6, 7, 8]);
            WriteModel("modc_test/mixed.nif", Glb(PaintMaterial, BrokenMaterial));
            AddBase("000001:Fixture.esm", "modc_test\\garbage.nif", 0.5f);
            AddBase("000002:Fixture.esm", "modc_test\\absent.nif", 0.5f);
            AddBase("000003:Fixture.esm", "modc_test\\mixed.nif", 0.5f);

            var summary = await RunAsync(TestContext.Current.CancellationToken);

            Assert.Equal(2, summary.UnreadableModels);
            Assert.Equal(1, summary.FailedBakes);
            Assert.Equal(1, summary.Synthesized);
            Assert.Equal(3, summary.Bases);
            Assert.Contains("2 unreadable models, 1 failed bakes", summary.ToString());
        }

        [Fact]
        public async Task RunAsync_NoTexturesOutput_CountsNoFailedBakes()
        {
            WriteModel("modc_test/model.nif", Glb(PaintMaterial));
            AddBase("000001:Fixture.esm", "modc_test\\model.nif", 0.5f);

            var summary = await ModcSwapSynthesizer.RunAsync(_db.Connection, null, _meshesOut, _staged.Root, null, TestContext.Current.CancellationToken);

            Assert.Equal(0, summary.FailedBakes);
        }

        [Fact]
        public async Task RunAsync_VariantWhoseSourceTextureIsMissing_DoesNotStopTheOtherBakesOrTheInserts()
        {
            WriteModel("modc_test/mixed.nif", Glb(PaintMaterial, BrokenMaterial));
            AddBase("000001:Fixture.esm", "modc_test\\mixed.nif", 0.5f);

            await RunAsync(TestContext.Current.CancellationToken);

            Assert.Equal(2, _db.QuerySingle<int>("SELECT COUNT(*) FROM material_swap_entries"));
            Assert.Equal([VariantFileName(PaletteDiffuse, 0.5f)], BakedFiles());
        }

        [Fact]
        public async Task RunAsync_NoTexturesOutput_StillInsertsTheSwapsAndBakesNothing()
        {
            WriteModel("modc_test/model.nif", Glb(PaintMaterial));
            AddBase("000001:Fixture.esm", "modc_test\\model.nif", 0.5f);

            await ModcSwapSynthesizer.RunAsync(_db.Connection, null, _meshesOut, _staged.Root, null, TestContext.Current.CancellationToken);

            Assert.Equal(["MODC:000001:Fixture.esm"], _db.Query<string>("SELECT form_key FROM material_swaps"));
            Assert.Empty(BakedFiles());
        }

        [Fact]
        public async Task RunAsync_BaseWithoutModcFloat_IsLeftAlone()
        {
            WriteModel("modc_test/model.nif", Glb(PaintMaterial));
            _db.Execute("INSERT INTO statics (form_key, model_path) VALUES ('000001:Fixture.esm', 'modc_test\\model.nif')");
            AddBaseSwap("000001:Fixture.esm", modc: null, swapFormKey: "000010:Fixture.esm");

            var summary = await RunAsync(TestContext.Current.CancellationToken);

            Assert.Equal(0, summary.Bases);
            Assert.Equal(["000010:Fixture.esm"], _db.Query<string>("SELECT material_swap FROM base_material_swaps"));
            Assert.Equal(0, _db.QuerySingle<int>("SELECT COUNT(*) FROM material_swaps"));
        }

        [Fact]
        public async Task RunAsync_SecondRun_SkipsTheBasesTheFirstRunSynthesized()
        {
            WriteModel("modc_test/model.nif", Glb(PaintMaterial));
            AddBase("000001:Fixture.esm", "modc_test\\model.nif", 0.5f);
            await RunAsync(TestContext.Current.CancellationToken);

            var second = await RunAsync(TestContext.Current.CancellationToken);

            Assert.Equal(0, second.Bases);
            Assert.Equal(1, _db.QuerySingle<int>("SELECT COUNT(*) FROM material_swap_entries"));
            Assert.Equal(["MODC:000001:Fixture.esm"], _db.Query<string>("SELECT material_swap FROM base_material_swaps"));
        }

        [Fact]
        public async Task Run_FinishedDatabaseOnDisk_CommitsTheSwapsThroughItsOwnConnection()
        {
            WriteModel("modc_test/model.nif", Glb(PaintMaterial));
            AddBase("000001:Fixture.esm", "modc_test\\model.nif", 0.5f);
            await _db.Connection.ExecuteAsync("PRAGMA wal_checkpoint(TRUNCATE)");

            var summary = ModcSwapSynthesizer.Run(_db.DatabasePath, _meshesOut, _staged.Root, _texturesOut, TestContext.Current.CancellationToken);

            Assert.Equal(1, summary.Synthesized);
            Assert.Equal(["MODC:000001:Fixture.esm"], _db.Query<string>("SELECT material_swap FROM base_material_swaps"));
            Assert.Equal([VariantFileName(PaletteDiffuse, 0.5f)], BakedFiles());
        }

        [Fact]
        public void Run_MissingDatabase_Throws()
        {
            string missing = Path.Combine(_output.Path, "missing.db");

            Assert.Throws<SqliteException>(() => ModcSwapSynthesizer.Run(missing, _meshesOut, _staged.Root, _texturesOut, TestContext.Current.CancellationToken));

            Assert.False(File.Exists(missing));
        }

        [Fact]
        public async Task RunAsync_Cancelled_ThrowsCancellationAndWritesNothing()
        {
            WriteModel("modc_test/model.nif", Glb(PaintMaterial));
            AddBase("000001:Fixture.esm", "modc_test\\model.nif", 0.5f);
            using var cts = new CancellationTokenSource();
            cts.Cancel();

            await Assert.ThrowsAnyAsync<OperationCanceledException>(() => RunAsync(cts.Token));

            Assert.Equal(0, _db.QuerySingle<int>("SELECT COUNT(*) FROM material_swaps"));
            Assert.Empty(BakedFiles());
        }
    }
}
