using System.Text.Json;
using AssetConverter;
using Mutagen.Bethesda.Fallout4;
using Xunit;

namespace InstallerTests
{
    /// <summary>
    /// Runs the real Fallout4.esm DOOR records and the game's mesh archives (install named by <c>OPENFO4_FO4_PATH</c>) through the door processor and
    /// <see cref="DoorTriggerBounds"/>, into a temporary database. Explicit: run with <c>-- xUnit.Explicit=only</c>.
    /// </summary>
    public sealed class DoorTriggerBoundsRealInstallTests : IAsyncLifetime
    {
        const string AutoloadDoor = "01ED77:Fallout4.esm";
        const string AutoloadDoorNoText = "07ECFC:Fallout4.esm";

        SchemaDatabase _db = null!;
        DoorTriggerBounds.Report _report = null!;

        public async ValueTask InitializeAsync()
        {
            string? game = Environment.GetEnvironmentVariable("OPENFO4_FO4_PATH");
            Assert.SkipWhen(string.IsNullOrEmpty(game), "OPENFO4_FO4_PATH names no Fallout 4 install.");
            using var file = Fallout4Mod.CreateFromBinaryOverlay(Path.Combine(game!, "Data", "Fallout4.esm"), Fallout4Release.Fallout4);

            _db = await SchemaDatabase.CreateAsync();
            await new DoorProcessor().ProcessRecordsAsync(file.Doors.ToList(), "DOOR", _db.Connection, silent: true);
            _report = DoorTriggerBounds.RunOnGame(_db.DatabasePath, Path.Combine(game!, "Data"));
        }

        public ValueTask DisposeAsync()
        {
            _db?.Dispose();
            return ValueTask.CompletedTask;
        }

        static void AssertAutoloadMarkerBox(string? json)
        {
            Assert.NotNull(json);
            using var box = JsonDocument.Parse(json);
            double[] Point(string corner) => new[] { "X", "Y", "Z" }.Select(axis => box.RootElement.GetProperty(corner).GetProperty(axis).GetDouble()).ToArray();
            Assert.Equal(new[] { -64.0, -8.0, 0.0 }, Point("Min"));
            Assert.Equal(new[] { 64.0, 8.0, 192.0 }, Point("Max"));
        }

        [Fact(Explicit = true)]
        public void TheAutoloadDoorsTriggerIsTheAutoloadMarkerGeometry()
        {
            AssertAutoloadMarkerBox(_db.QuerySingle<string?>("SELECT trigger_bounds FROM doors WHERE form_key = @key", new { key = AutoloadDoor }));
            AssertAutoloadMarkerBox(_db.QuerySingle<string?>("SELECT trigger_bounds FROM doors WHERE form_key = @key", new { key = AutoloadDoorNoText }));
        }

        [Fact(Explicit = true)]
        public void OnlyAutomaticDoorsHaveTriggerBoundsAndNoneFailed()
        {
            var withBounds = _db.Query<string>("SELECT form_key FROM doors WHERE trigger_bounds IS NOT NULL ORDER BY form_key");

            Assert.All(withBounds, formKey => Assert.True(
                DoorTriggerBounds.IsAutomatic(_db.QuerySingle<string?>("SELECT flags FROM doors WHERE form_key = @formKey", new { formKey }))));
            Assert.Empty(_report.Failures);
            Assert.Equal(withBounds.Count, _report.Filled);
        }
    }
}
