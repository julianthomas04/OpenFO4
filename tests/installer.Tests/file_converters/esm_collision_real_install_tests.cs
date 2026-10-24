using System.Text.Json;
using AssetConverter;
using Dapper;
using Microsoft.Data.Sqlite;
using Mutagen.Bethesda.Fallout4;
using Xunit;

namespace InstallerTests
{
    /// <summary>
    /// Runs the real Fallout4.esm COLL records (install named by <c>OPENFO4_FO4_PATH</c>) through <see cref="CollisionLayerProcessor"/> into an
    /// in-memory database and prints every layer to the test output. Explicit: run with <c>-- xUnit.Explicit=only</c>.
    /// </summary>
    public sealed class EsmCollisionRealInstallTests : IAsyncLifetime
    {
        const string NavCut = "L_NAVCUT";
        const string CharController = "L_CHARCONTROLLER";
        const string CharBumper = "L_CHARBUMPER";
        const long NavCutIndex = 49;

        sealed class LayerRow
        {
            public string? EditorId { get; init; }
            public long? LayerIndex { get; init; }
            public long? Flags { get; init; }
            public string? CollidesWith { get; init; }
        }

        readonly Xunit.ITestOutputHelper _output;
        SqliteConnection _conn = null!;
        List<LayerRow> _layers = [];

        public EsmCollisionRealInstallTests(Xunit.ITestOutputHelper output) => _output = output;

        public async ValueTask InitializeAsync()
        {
            string? game = Environment.GetEnvironmentVariable("OPENFO4_FO4_PATH");
            Assert.SkipWhen(string.IsNullOrEmpty(game), "OPENFO4_FO4_PATH names no Fallout 4 install.");
            using var file = Fallout4Mod.CreateFromBinaryOverlay(Path.Combine(game!, "Data", "Fallout4.esm"), Fallout4Release.Fallout4);

            _conn = new SqliteConnection("Data Source=:memory:");
            await _conn.OpenAsync();
            await _conn.ExecuteAsync(await File.ReadAllTextAsync(InstallerPaths.Require("src/installer/file_converters/esm_schema.sql")));
            await new CollisionLayerProcessor().ProcessRecordsAsync(file.CollisionLayers.ToList(), "COLL", _conn, silent: true);
            _layers = (await _conn.QueryAsync<LayerRow>("SELECT editor_id AS EditorId, layer_index AS LayerIndex, flags AS Flags, collides_with AS CollidesWith FROM collision_layers ORDER BY layer_index")).ToList();
        }

        public ValueTask DisposeAsync()
        {
            _conn?.Dispose();
            return ValueTask.CompletedTask;
        }

        LayerRow Layer(string editorId) => _layers.Single(layer => layer.EditorId == editorId);

        [Fact(Explicit = true)]
        public void NavCutHasTheDocumentedLayerIndex()
        {
            Assert.Equal(NavCutIndex, Layer(NavCut).LayerIndex);
        }

        [Fact(Explicit = true)]
        public void TheCharacterLayersExist()
        {
            Assert.NotNull(Layer(CharController));
            Assert.NotNull(Layer(CharBumper));
        }

        [Fact(Explicit = true)]
        public void ReportEveryLayerAndWhetherItCollidesWithTheCharacterLayers()
        {
            string KeyOf(string editorId) => _conn.QuerySingle<string>("SELECT form_key FROM collision_layers WHERE editor_id = @editorId", new { editorId });
            var characterKeys = new[] { KeyOf(CharController), KeyOf(CharBumper) };

            foreach (var layer in _layers)
            {
                string[]? collidesWith = layer.CollidesWith is null ? null : JsonSerializer.Deserialize<string[]>(layer.CollidesWith);
                string hitsCharacter = collidesWith is null ? "UNKNOWN (collides_with NULL)" : collidesWith.Intersect(characterKeys).Any().ToString();
                _output.WriteLine($"{layer.EditorId}\tindex={layer.LayerIndex}\tflags={layer.Flags}\tcollides_with_character={hitsCharacter}");
            }

            Assert.NotEmpty(_layers);
        }
    }
}
