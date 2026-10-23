using AssetConverter;
using Xunit;

namespace InstallerTests
{
    // Convention (lod_backfill.cs): Meshes\Foo\Bar.nif -> Meshes\LOD\Foo\Bar_LOD.nif; multi-level LODs are
    // Bar_LOD_0 (closest) .. Bar_LOD_3 and the highest-detail one wins: _LOD_0 > bare _LOD > _LOD_N.
    public sealed class LodBackfillTests : IDisposable
    {
        readonly SchemaDatabase _db = SchemaDatabase.CreateAsync().GetAwaiter().GetResult();
        readonly string _meshesOut;

        public LodBackfillTests()
        {
            _meshesOut = Path.Combine(_db.DirectoryPath, InstallLayout.MeshesFolder);
            Directory.CreateDirectory(Path.Combine(_meshesOut, "LOD"));
        }

        public void Dispose() => _db.Dispose();

        void AddLodGlb(string relativeToLodDirectory)
        {
            string path = Path.Combine(_meshesOut, "LOD", relativeToLodDirectory);
            Directory.CreateDirectory(Path.GetDirectoryName(path)!);
            File.WriteAllBytes(path, []);
        }

        void AddStatic(string formKey, string? modelPath, string? editorId = null) =>
            _db.Execute(
                "INSERT INTO statics (form_key, editor_id, model_path) VALUES (@formKey, @editorId, @modelPath)",
                new { formKey, editorId, modelPath });

        string? LodPathOf(string formKey) =>
            _db.QuerySingle<string?>("SELECT lod_model_path FROM statics WHERE form_key = @formKey", new { formKey });

        void Run() => LodBackfill.Run(_db.DatabasePath, _meshesOut);

        [Fact]
        public void Run_ModelMirroredUnderLodDirectory_GetsStoredBackslashNifPath()
        {
            AddLodGlb(Path.Combine("Architecture", "Foo", "Bar_LOD.glb"));
            AddStatic("000001:A.esm", @"architecture\foo\bar.nif");

            Run();

            Assert.Equal(@"LOD\Architecture\Foo\Bar_LOD.nif", LodPathOf("000001:A.esm"));
        }

        [Fact]
        public void Run_PathMatch_IsCaseInsensitive()
        {
            AddLodGlb(Path.Combine("Architecture", "Foo", "Bar_LOD.glb"));
            AddStatic("000001:A.esm", @"Architecture\FOO\BAR.NIF");

            Run();

            Assert.Equal(@"LOD\Architecture\Foo\Bar_LOD.nif", LodPathOf("000001:A.esm"));
        }

        [Fact]
        public void Run_SeveralLodLevels_PrefersLevelZeroOverBareOverHigherLevels()
        {
            AddLodGlb("Zed_LOD_2.glb");
            AddLodGlb("Zed_LOD.glb");
            AddLodGlb("Zed_LOD_0.glb");
            AddLodGlb("Yes_LOD_3.glb");
            AddLodGlb("Yes_LOD_1.glb");
            AddLodGlb("Bare_LOD.glb");
            AddLodGlb("Bare_LOD_2.glb");
            AddStatic("000001:A.esm", "zed.nif");
            AddStatic("000002:A.esm", "yes.nif");
            AddStatic("000003:A.esm", "bare.nif");

            Run();

            Assert.Equal(@"LOD\Zed_LOD_0.nif", LodPathOf("000001:A.esm"));
            Assert.Equal(@"LOD\Yes_LOD_1.nif", LodPathOf("000002:A.esm"));
            Assert.Equal(@"LOD\Bare_LOD.nif", LodPathOf("000003:A.esm"));
        }

        [Fact]
        public void Run_ModelPathMismatch_FallsBackToEditorIdMatchOnTheLodFileName()
        {
            AddLodGlb(Path.Combine("Somewhere", "Rock01_LOD.glb"));
            AddStatic("000001:A.esm", @"completely\different\model.nif", "Rock01");

            Run();

            Assert.Equal(@"LOD\Somewhere\Rock01_LOD.nif", LodPathOf("000001:A.esm"));
        }

        [Fact]
        public void Run_EditorIdFallback_StripsATrailingLodSuffixAndIgnoresCase()
        {
            AddLodGlb(Path.Combine("Somewhere", "Rock01_LOD.glb"));
            AddStatic("000001:A.esm", @"other\model.nif", "ROCK01_lod");

            Run();

            Assert.Equal(@"LOD\Somewhere\Rock01_LOD.nif", LodPathOf("000001:A.esm"));
        }

        [Fact]
        public void Run_NoMatchAtAll_LeavesLodPathNull()
        {
            AddLodGlb("Unrelated_LOD.glb");
            AddStatic("000001:A.esm", @"architecture\foo\bar.nif", "Bar");

            Run();

            Assert.Null(LodPathOf("000001:A.esm"));
        }

        [Fact]
        public void Run_StaticWhoseModelIsAlreadyUnderLod_IsSkipped()
        {
            AddLodGlb(Path.Combine("Foo", "Bar_LOD.glb"));
            AddLodGlb(Path.Combine("Foo", "Bar_LOD_LOD.glb"));
            AddStatic("000001:A.esm", @"LOD\Foo\Bar_LOD.nif", "Bar");

            Run();

            Assert.Null(LodPathOf("000001:A.esm"));
        }

        [Theory]
        [InlineData(@"LOD\Foo\Bar_LOD.nif")]
        [InlineData("LOD/Foo/Bar_LOD.nif")]
        [InlineData(@"lod\foo\bar_lod.nif")]
        [InlineData(@"Lod/Foo/Bar_LOD.nif")]
        public void Run_ModelUnderLodInEitherSlashAndAnyCase_IsSkipped(string modelPath)
        {
            AddLodGlb(Path.Combine("Foo", "Bar_LOD.glb"));
            AddStatic("000001:A.esm", modelPath, "Bar");

            Run();

            Assert.Null(LodPathOf("000001:A.esm"));
        }

        [Theory]
        [InlineData(@"Lodge\Bar.nif")]
        [InlineData("LODBar.nif")]
        [InlineData(@"Foo\LOD\Bar.nif")]
        public void Run_ModelOnlyResemblingTheLodDirectory_IsStillBackfilled(string modelPath)
        {
            AddLodGlb(Path.Combine("Foo", "Bar_LOD.glb"));
            AddStatic("000001:A.esm", modelPath, "Bar");

            Run();

            Assert.Equal(@"LOD\Foo\Bar_LOD.nif", LodPathOf("000001:A.esm"));
        }

        [Fact]
        public void Run_StaticWithoutModel_IsSkipped()
        {
            AddLodGlb("Anything_LOD.glb");
            AddStatic("000001:A.esm", null, "Anything");

            Run();

            Assert.Null(LodPathOf("000001:A.esm"));
        }

        [Fact]
        public void Run_GlbWithoutLodSuffix_IsNotACandidate()
        {
            AddLodGlb(Path.Combine("Foo", "Bar.glb"));
            AddStatic("000001:A.esm", @"foo\bar.nif");

            Run();

            Assert.Null(LodPathOf("000001:A.esm"));
        }

        [Fact]
        public void Run_WithoutLodDirectory_ChangesNothing()
        {
            Directory.Delete(Path.Combine(_meshesOut, "LOD"));
            AddStatic("000001:A.esm", @"foo\bar.nif");

            Run();

            Assert.Null(LodPathOf("000001:A.esm"));
        }

        [Fact]
        public void Run_WithoutDatabaseFile_DoesNotThrow()
        {
            string missing = Path.Combine(_db.DirectoryPath, "missing.db");

            LodBackfill.Run(missing, _meshesOut);

            Assert.False(File.Exists(missing));
        }
    }
}
