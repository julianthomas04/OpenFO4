using System.Numerics;
using AssetConverter;
using InstallerTests.Nif;
using NiflySharp.Structs;
using Xunit;

namespace InstallerTests
{
    public sealed class DoorTriggerBoundsTests : IAsyncLifetime
    {
        const string AutomaticFlags = "2";
        const string Marker = """{"Min":{"X":-64,"Y":-8,"Z":0},"Max":{"X":64,"Y":8,"Z":192}}""";
        static readonly Triangle[] OneTriangle = [new Triangle(0, 1, 2)];

        SchemaDatabase _db = null!;

        public async ValueTask InitializeAsync() => _db = await SchemaDatabase.CreateAsync();

        public ValueTask DisposeAsync()
        {
            _db.Dispose();
            return ValueTask.CompletedTask;
        }

        void AddDoor(string formKey, string? model, string? flags) =>
            _db.Execute("INSERT INTO doors (form_key, editor_id, model_path, flags) VALUES (@formKey, @formKey, @model, @flags)", new { formKey, model, flags });

        string? TriggerBounds(string formKey) =>
            _db.QuerySingle<string?>("SELECT trigger_bounds FROM doors WHERE form_key = @formKey", new { formKey });

        static byte[] NifWithShape(string shapeName, Vector3[] positions)
        {
            using var directory = new TempDirectory();
            var scene = new NifSceneBuilder();
            scene.AddShape(scene.Root, shapeName, positions, OneTriangle);
            return File.ReadAllBytes(scene.Save(directory.Path, "model.nif"));
        }

        static byte[] MarkerNif() => NifWithShape("EditorMarker", [new(-64, -8, 0), new(64, 8, 192), new(-64, 8, 0)]);

        [Fact]
        public void AnAutomaticDoorGetsTheBoundsOfItsModelsEditorMarker()
        {
            AddDoor("A", "marker.nif", AutomaticFlags);
            var nif = MarkerNif();

            var report = DoorTriggerBounds.Run(_db.DatabasePath, model => model == "marker.nif" ? nif : null);

            Assert.Equal(Marker, TriggerBounds("A"));
            Assert.Equal(1, report.Filled);
            Assert.Empty(report.Failures);
        }

        [Theory]
        [InlineData("0")]
        [InlineData("64")]
        [InlineData(null)]
        public void ADoorWithoutTheAutomaticFlagIsLeftAloneAndItsModelIsNotRead(string? flags)
        {
            AddDoor("A", "marker.nif", flags);
            var read = new List<string>();

            var report = DoorTriggerBounds.Run(_db.DatabasePath, model => { read.Add(model); return null; });

            Assert.Null(TriggerBounds("A"));
            Assert.Empty(read);
            Assert.Empty(report.Failures);
        }

        [Fact]
        public void AnAutomaticFlagAmongOthersCounts()
        {
            AddDoor("A", "marker.nif", "66");
            var nif = MarkerNif();

            DoorTriggerBounds.Run(_db.DatabasePath, _ => nif);

            Assert.Equal(Marker, TriggerBounds("A"));
        }

        [Fact]
        public void ASharedModelIsReadOnce()
        {
            AddDoor("A", "marker.nif", "2");
            AddDoor("B", "MARKER.nif", "66");
            var nif = MarkerNif();
            int reads = 0;

            DoorTriggerBounds.Run(_db.DatabasePath, _ => { reads++; return nif; });

            Assert.Equal(1, reads);
            Assert.Equal(Marker, TriggerBounds("B"));
        }

        [Fact]
        public void AMissingModelLeavesNullAndIsReportedWithTheDoor()
        {
            AddDoor("A", "gone.nif", AutomaticFlags);

            var report = DoorTriggerBounds.Run(_db.DatabasePath, _ => null);

            Assert.Null(TriggerBounds("A"));
            var failure = Assert.Single(report.Failures);
            Assert.Contains("A", failure);
            Assert.Contains("gone.nif", failure);
        }

        [Fact]
        public void AModelWithoutMarkerGeometryLeavesNullAndIsReported()
        {
            AddDoor("A", "plain.nif", AutomaticFlags);
            var nif = NifWithShape("Door", [new(0, 0, 0), new(1, 0, 0), new(0, 1, 0)]);

            var report = DoorTriggerBounds.Run(_db.DatabasePath, _ => nif);

            Assert.Null(TriggerBounds("A"));
            Assert.Contains("no EditorMarker geometry", Assert.Single(report.Failures));
        }

        [Fact]
        public void AnUnreadableModelLeavesNullAndIsReported()
        {
            AddDoor("A", "broken.nif", AutomaticFlags);

            var report = DoorTriggerBounds.Run(_db.DatabasePath, _ => [1, 2, 3, 4]);

            Assert.Null(TriggerBounds("A"));
            Assert.Contains("could not be read", Assert.Single(report.Failures));
        }

        [Fact]
        public void AnAutomaticDoorWithoutAModelIsReported()
        {
            AddDoor("A", null, AutomaticFlags);

            var report = DoorTriggerBounds.Run(_db.DatabasePath, _ => throw new InvalidOperationException("no read expected"));

            Assert.Contains("has no model", Assert.Single(report.Failures));
        }
    }
}
