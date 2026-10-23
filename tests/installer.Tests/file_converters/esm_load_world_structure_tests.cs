using System.Text.Json;
using Xunit;

namespace InstallerTests
{
    // Cells, worldspaces, load-door teleports and landscapes: the rows the engine's streamer and activation map read
    // (src/engine/src/assets/db/{cells,worldspaces,teleports,landscapes}.rs).
    [Collection(FixtureEsm.Name)]
    public class EsmLoadWorldStructureTests(LoadedFixtureDatabase db)
    {
        CellRow Cell(string editorId) =>
            db.QuerySingle<CellRow>("SELECT * FROM cells WHERE form_key = @key", new { key = db.KeyOf(editorId) });

        [Fact]
        public void Cell_ExteriorSubCell_KeepsGridAndWorldspace()
        {
            var cell = Cell("FixtureExterior");

            Assert.Equal((TinyPlugin.ExteriorGridX, TinyPlugin.ExteriorGridY), (cell.GridX, cell.GridY));
            Assert.Equal(db.KeyOf("FixtureWorld"), cell.Worldspace);
            Assert.Equal(0, cell.IsInterior);
            Assert.Equal(0, cell.IsWorldspacePersistentCell);
        }

        [Fact]
        public void Cell_WorldspaceTopCell_IsFlaggedPersistentSoGridQueriesCanExcludeIt()
        {
            var cell = Cell("FixtureWorldTopCell");

            Assert.Equal(1, cell.IsWorldspacePersistentCell);
            Assert.Equal(db.KeyOf("FixtureWorld"), cell.Worldspace);
        }

        [Fact]
        public void Cell_Interior_HasNoWorldspaceOrGrid()
        {
            var cell = Cell("FixtureInterior");

            Assert.Equal(1, cell.IsInterior);
            Assert.Null(cell.Worldspace);
            Assert.Null(cell.GridX);
            Assert.Null(cell.GridY);
        }

        [Fact]
        public void Cell_InteriorLighting_IsEngineReadableJson()
        {
            using var lighting = JsonDocument.Parse(Cell("FixtureInterior").Lighting!);
            var root = lighting.RootElement;

            Assert.Equal(10, root.GetProperty("AmbientColor").GetProperty("R").GetInt32());
            Assert.Equal(20, root.GetProperty("AmbientColor").GetProperty("G").GetInt32());
            Assert.Equal(30, root.GetProperty("AmbientColor").GetProperty("B").GetInt32());
            Assert.Equal(200, root.GetProperty("DirectionalColor").GetProperty("R").GetInt32());
            Assert.Equal(64.0, root.GetProperty("FogNear").GetDouble());
            Assert.Equal(2048.0, root.GetProperty("FogFar").GetDouble());
            Assert.Equal(45, root.GetProperty("DirectionalRotationXY").GetInt32());
            Assert.Equal(-30, root.GetProperty("DirectionalRotationZ").GetInt32());
        }

        [Fact]
        public void Worldspace_Record_IsStoredWithEditorId()
        {
            var editorId = db.QuerySingle<string>("SELECT editor_id FROM worldspaces WHERE form_key = @key", new { key = db.KeyOf("FixtureWorld") });

            Assert.Equal("FixtureWorld", editorId);
        }

        [Fact]
        public void Cells_AreRegisteredInRecordsTableWithCellType()
        {
            var types = db.Query<string>(
                "SELECT record_type FROM records WHERE form_key IN (@a, @b, @c)",
                new { a = db.KeyOf("FixtureExterior"), b = db.KeyOf("FixtureInterior"), c = db.KeyOf("FixtureWorldTopCell") });

            Assert.Equal(["CELL", "CELL", "CELL"], types);
        }

        // The same JOIN as TeleportRecord's SELECT_RESOLVED in src/engine/src/assets/db/teleports.rs: a door only
        // becomes activatable when its destination ref resolves to a cell.
        const string EngineResolvedTeleportQuery = @"
            SELECT t.source_ref AS SourceRef, t.dest_position_x AS DestX, t.dest_position_y AS DestY,
                   t.dest_position_z AS DestZ, t.dest_rotation_z AS DestRotZ,
                   c.is_interior AS IsInterior, c.worldspace AS Worldspace, c.form_key AS DestCell
            FROM placed_teleports t
            JOIN placed_objects po ON po.form_key = t.destination_ref
            JOIN cells c ON c.form_key = po.cell_form_key";

        sealed class TeleportRow
        {
            public required string SourceRef { get; init; }
            public double DestX { get; init; }
            public double DestY { get; init; }
            public double DestZ { get; init; }
            public double DestRotZ { get; init; }
            public long IsInterior { get; init; }
            public string? Worldspace { get; init; }
            public required string DestCell { get; init; }
        }

        [Fact]
        public void Teleport_ExteriorDoor_ResolvesToInteriorCellWithDestinationTransform()
        {
            var door = db.Query<TeleportRow>(EngineResolvedTeleportQuery).Single(t => t.SourceRef == db.KeyOf("ExteriorDoor"));

            Assert.Equal(db.KeyOf("FixtureInterior"), door.DestCell);
            Assert.Equal(1, door.IsInterior);
            Assert.Null(door.Worldspace);
            Assert.Equal((-1.0, -2.0, -3.0), (door.DestX, door.DestY, door.DestZ));
        }

        [Fact]
        public void Teleport_InteriorDoor_ResolvesToExteriorCellInWorldspace()
        {
            var door = db.Query<TeleportRow>(EngineResolvedTeleportQuery).Single(t => t.SourceRef == db.KeyOf("InteriorDoor"));

            Assert.Equal(db.KeyOf("FixtureExterior"), door.DestCell);
            Assert.Equal(0, door.IsInterior);
            Assert.Equal(db.KeyOf("FixtureWorld"), door.Worldspace);
            Assert.Equal((10.0, 20.0, 30.0), (door.DestX, door.DestY, door.DestZ));
            Assert.Equal(1.5, door.DestRotZ);
        }

        [Fact]
        public void Teleport_RefWithoutDestinationDoor_HasNoRow()
        {
            var count = db.QuerySingle<long>("SELECT COUNT(*) FROM placed_teleports WHERE source_ref = @key", new { key = db.KeyOf("PlainRef") });

            Assert.Equal(0, count);
        }

        [Fact]
        public void Teleport_OnlyTheTwoLoadDoorsHaveRows() =>
            Assert.Equal(2, db.QuerySingle<long>("SELECT COUNT(*) FROM placed_teleports"));

        sealed class LandscapeRow
        {
            public required string FormKey { get; init; }
            public string? Cell { get; init; }
            public required byte[] VertexHeightMap { get; init; }
            public byte[]? VertexColors { get; init; }
            public byte[]? Layers { get; init; }
        }

        LandscapeRow Landscape() => db.QuerySingle<LandscapeRow>("SELECT * FROM landscapes");

        [Fact]
        public void Landscape_IsLinkedToItsCell() =>
            Assert.Equal(db.KeyOf("FixtureExterior"), Landscape().Cell);

        // 1093 bytes, f32 offset then 33 x 33 i8 deltas, row-major (y, x).
        [Fact]
        public void Landscape_HeightMap_IsOffsetPlusRowMajorSignedDeltas()
        {
            var blob = Landscape().VertexHeightMap;

            Assert.Equal(4 + 33 * 33, blob.Length);
            Assert.Equal(42.5f, BitConverter.ToSingle(blob, 0));
            Assert.Equal(5, (sbyte)blob[4 + 2 * 33 + 3]);
            Assert.Equal(-7, (sbyte)blob[4 + 32 * 33 + 32]);
        }

        // 3267 bytes, 33 x 33 RGB, row-major.
        [Fact]
        public void Landscape_VertexColors_AreRowMajorRgb()
        {
            var blob = Landscape().VertexColors!;

            Assert.Equal(33 * 33 * 3, blob.Length);
            Assert.Equal(new byte[] { 10, 20, 30 }, blob[(1 * 3)..(1 * 3 + 3)]);
            Assert.Equal(new byte[] { 40, 50, 60 }, blob[(33 * 3)..(33 * 3 + 3)]);
        }

        // u16 count, then per layer NUL-terminated texture key, u8 quadrant,
        // u8 flags (bit 0 = base layer), u16 alpha length, alpha bytes.
        [Fact]
        public void Landscape_Layers_UseDocumentedBlobLayout()
        {
            var blob = Landscape().Layers!;
            var textureKey = System.Text.Encoding.UTF8.GetBytes(db.KeyOf("GrassTexture"));
            var reader = new BinaryReader(new MemoryStream(blob));

            Assert.Equal(2, reader.ReadUInt16());

            Assert.Equal(textureKey, reader.ReadBytes(textureKey.Length));
            Assert.Equal(0, reader.ReadByte());
            Assert.Equal((byte)Mutagen.Bethesda.Plugins.Records.Quadrant.BottomLeft, reader.ReadByte());
            Assert.Equal(1, reader.ReadByte());
            Assert.Equal(0, reader.ReadUInt16());

            Assert.Equal(textureKey, reader.ReadBytes(textureKey.Length));
            Assert.Equal(0, reader.ReadByte());
            Assert.Equal((byte)Mutagen.Bethesda.Plugins.Records.Quadrant.TopRight, reader.ReadByte());
            Assert.Equal(0, reader.ReadByte());
            Assert.Equal(6, reader.ReadUInt16());
            Assert.Equal(new byte[] { 1, 2, 3, 4, 5, 6 }, reader.ReadBytes(6));
            Assert.Equal(blob.Length, reader.BaseStream.Position);
        }
    }
}
