using AssetConverter;
using Xunit;

namespace InstallerTests
{
    // Step 3b of the install: the engine looks GLBs up by lowercase model path (docs/installer/database.md).
    public sealed class ModelPathNormalizerTests : IDisposable
    {
        readonly SchemaDatabase _db = SchemaDatabase.CreateAsync().GetAwaiter().GetResult();

        public void Dispose() => _db.Dispose();

        void InsertStatic(string formKey, string? modelPath, string? lodModelPath = null, string editorId = "MixedCaseEditorId") =>
            _db.Execute(
                "INSERT INTO statics (form_key, editor_id, model_path, lod_model_path) VALUES (@formKey, @editorId, @modelPath, @lodModelPath)",
                new { formKey, editorId, modelPath, lodModelPath });

        string? ModelPath(string formKey) =>
            _db.QuerySingle<string?>("SELECT model_path FROM statics WHERE form_key = @formKey", new { formKey });

        string? LodModelPath(string formKey) =>
            _db.QuerySingle<string?>("SELECT lod_model_path FROM statics WHERE form_key = @formKey", new { formKey });

        [Fact]
        public void Run_MixedCaseModelPath_IsLowercased()
        {
            InsertStatic("000001:A.esm", @"Architecture\Foo\Bar.NIF");

            ModelPathNormalizer.Run(_db.DatabasePath);

            Assert.Equal(@"architecture\foo\bar.nif", ModelPath("000001:A.esm"));
        }

        [Fact]
        public void Run_LodModelPath_IsLowercasedToo()
        {
            InsertStatic("000001:A.esm", "a.nif", @"LOD\Foo\Bar_LOD.nif");

            ModelPathNormalizer.Run(_db.DatabasePath);

            Assert.Equal(@"lod\foo\bar_lod.nif", LodModelPath("000001:A.esm"));
        }

        [Fact]
        public void Run_NullPaths_StayNull()
        {
            InsertStatic("000001:A.esm", null);

            ModelPathNormalizer.Run(_db.DatabasePath);

            Assert.Null(ModelPath("000001:A.esm"));
            Assert.Null(LodModelPath("000001:A.esm"));
        }

        [Fact]
        public void Run_OtherColumns_KeepTheirCase()
        {
            InsertStatic("000001:A.esm", "X.nif", editorId: "KeepMyCase");

            ModelPathNormalizer.Run(_db.DatabasePath);

            var editorId = _db.QuerySingle<string>("SELECT editor_id FROM statics WHERE form_key = '000001:A.esm'");
            Assert.Equal("KeepMyCase", editorId);
        }

        [Fact]
        public void Run_ReturnsNumberOfRowsActuallyChangedAndIsIdempotent()
        {
            InsertStatic("000001:A.esm", "Upper.nif");
            InsertStatic("000002:A.esm", "lower.nif");
            InsertStatic("000003:A.esm", "Mixed.nif", "LOD\\Mixed_LOD.nif");

            int first = ModelPathNormalizer.Run(_db.DatabasePath);
            int second = ModelPathNormalizer.Run(_db.DatabasePath);

            Assert.Equal(3, first);
            Assert.Equal(0, second);
        }

        [Fact]
        public void Run_WithTableFilter_LeavesOtherTablesUntouched()
        {
            InsertStatic("000001:A.esm", "Upper.nif");
            _db.Execute("INSERT INTO doors (form_key, editor_id, model_path) VALUES ('000002:A.esm', 'D', 'Door.NIF')");

            ModelPathNormalizer.Run(_db.DatabasePath, ["doors"]);

            Assert.Equal("Upper.nif", ModelPath("000001:A.esm"));
            Assert.Equal("door.nif", _db.QuerySingle<string>("SELECT model_path FROM doors"));
        }
    }
}
