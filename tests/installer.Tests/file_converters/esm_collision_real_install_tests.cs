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
    /// in-memory database and checks known facts of the layers. Explicit: run with <c>-- xUnit.Explicit=only</c>.
    /// </summary>
    public sealed class EsmCollisionRealInstallTests : IAsyncLifetime
    {
        const string NavCut = "L_NAVCUT";
        const string CharController = "L_CHARCONTROLLER";
        const string CharBumper = "L_CHARBUMPER";
        const int LayerCount = 57;

        sealed class LayerRow
        {
            public string FormKey { get; init; } = "";
            public string? EditorId { get; init; }
            public long? LayerIndex { get; init; }
            public long? Flags { get; init; }
            public string? CollidesWith { get; init; }
        }

        SqliteConnection _conn = null!;
        List<LayerRow> _layers = [];

        public async ValueTask InitializeAsync()
        {
            string? game = Environment.GetEnvironmentVariable("OPENFO4_FO4_PATH");
            Assert.SkipWhen(string.IsNullOrEmpty(game), "OPENFO4_FO4_PATH names no Fallout 4 install.");
            using var file = Fallout4Mod.CreateFromBinaryOverlay(Path.Combine(game!, "Data", "Fallout4.esm"), Fallout4Release.Fallout4);

            _conn = new SqliteConnection("Data Source=:memory:");
            await _conn.OpenAsync();
            await _conn.ExecuteAsync(await File.ReadAllTextAsync(InstallerPaths.Require("src/installer/file_converters/esm_schema.sql")));
            await new CollisionLayerProcessor().ProcessRecordsAsync(file.CollisionLayers.ToList(), "COLL", _conn, silent: true);
            _layers = (await _conn.QueryAsync<LayerRow>("SELECT form_key AS FormKey, editor_id AS EditorId, layer_index AS LayerIndex, flags AS Flags, collides_with AS CollidesWith FROM collision_layers ORDER BY layer_index")).ToList();
        }

        public ValueTask DisposeAsync()
        {
            _conn?.Dispose();
            return ValueTask.CompletedTask;
        }

        LayerRow Layer(string editorId) => _layers.Single(layer => layer.EditorId == editorId);

        static string[] KeysOf(LayerRow layer) => layer.CollidesWith is null ? [] : JsonSerializer.Deserialize<string[]>(layer.CollidesWith)!;

        [Theory(Explicit = true)]
        [InlineData("L_STATIC", 1)]
        [InlineData("L_TRIGGER", 12)]
        [InlineData(CharController, 30)]
        [InlineData(NavCut, 49)]
        [InlineData(CharBumper, 56)]
        public void ALayerHasItsKnownIndex(string editorId, long index)
        {
            Assert.Equal(index, Layer(editorId).LayerIndex);
        }

        [Fact(Explicit = true)]
        public void TheLayerIndicesCountUpWithoutGaps()
        {
            Assert.Equal(Enumerable.Range(0, LayerCount).Select(index => (long?)index), _layers.Select(layer => layer.LayerIndex));
        }

        [Theory(Explicit = true)]
        [InlineData("L_STATIC", (CollisionLayer.Flag)0)]
        [InlineData("L_TRIGGER", CollisionLayer.Flag.TriggerVolume | CollisionLayer.Flag.Sensor)]
        [InlineData(NavCut, CollisionLayer.Flag.TriggerVolume | CollisionLayer.Flag.NavmeshObstacle)]
        [InlineData(CharController, (CollisionLayer.Flag)0)]
        public void ALayerHasItsKnownFlags(string editorId, CollisionLayer.Flag flags)
        {
            Assert.Equal((long)flags, Layer(editorId).Flags);
        }

        [Fact(Explicit = true)]
        public void NavCutHasNoCollidesWithList()
        {
            Assert.Null(Layer(NavCut).CollidesWith);
        }

        [Theory(Explicit = true)]
        [InlineData("L_STATIC")]
        [InlineData("L_STAIRHELPER")]
        [InlineData("L_TERRAIN")]
        [InlineData("L_TRIGGER")]
        public void ALayerTheCharacterStandsOnOrEntersCollidesWithTheCharacterController(string editorId)
        {
            Assert.Contains(Layer(CharController).FormKey, KeysOf(Layer(editorId)));
        }

        [Fact(Explicit = true)]
        public void TheCharacterBumperCollidesOnlyWithTheCharacterController()
        {
            Assert.Equal([Layer(CharController).FormKey], KeysOf(Layer(CharBumper)));
        }

        [Theory(Explicit = true)]
        [InlineData(CharController)]
        [InlineData(CharBumper)]
        public void ACharacterLayerListsExactlyTheLayersThatListIt(string characterEditorId)
        {
            var character = Layer(characterEditorId);
            var listingTheCharacter = _layers.Where(layer => KeysOf(layer).Contains(character.FormKey)).Select(layer => layer.FormKey);

            Assert.Equal(KeysOf(character).Order(), listingTheCharacter.Order());
        }
    }
}
