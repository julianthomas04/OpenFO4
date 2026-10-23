using AssetConverter;
using AssetConverter.Pipeline;
using Dapper;
using Microsoft.Data.Sqlite;
using Mutagen.Bethesda.Fallout4;
using Mutagen.Bethesda.Plugins;
using Xunit;

namespace InstallerTests
{
    // Navigation mesh extraction contract.
    [Collection(FixtureEsm.Name)]
    public class EsmLoadNavMeshTests(LoadedFixtureDatabase db)
    {
        sealed class NavMeshRecord
        {
            public string? EditorId { get; init; }
            public long FormId { get; init; }
            public string? Cell { get; init; }
            public string? Worldspace { get; init; }
            public long? GridX { get; init; }
            public long? GridY { get; init; }
            public long RecordFlags { get; init; }
            public long FormatVersion { get; init; }
            public long VertexCount { get; init; }
            public long TriangleCount { get; init; }
            public long EdgeLinkCount { get; init; }
            public long DoorLinkCount { get; init; }
            public byte[] Vertices { get; init; } = [];
            public byte[] Triangles { get; init; } = [];
            public byte[] EdgeLinks { get; init; } = [];
            public byte[] DoorLinks { get; init; } = [];
        }

        NavMeshRecord Row(string name) =>
            db.QuerySingle<NavMeshRecord>("SELECT * FROM navigation_meshes WHERE form_key = @key", new { key = db.KeyOf(name) });

        uint IdOf(string name) => FormKey.Factory(db.KeyOf(name)).ID;

        static byte[] Bytes(params object[] values) => values.SelectMany(value => value switch
        {
            float f => BitConverter.GetBytes(f),
            ushort u => BitConverter.GetBytes(u),
            short s => BitConverter.GetBytes(s),
            uint i => BitConverter.GetBytes(i),
            byte b => [b],
            _ => throw new ArgumentException(value.GetType().Name),
        }).ToArray();

        [Fact]
        public void ExteriorMesh_StoresOwnerCellWorldspaceAndGrid()
        {
            var row = Row(NavMeshFixture.Linked);

            Assert.Equal(db.KeyOf(TinyPlugin.ExteriorCellEditorId), row.Cell);
            Assert.Equal(db.KeyOf("FixtureWorld"), row.Worldspace);
            Assert.Equal(IdOf(NavMeshFixture.Linked), (uint)row.FormId);
            Assert.Equal(15, row.FormatVersion);
            Assert.Equal((4L, 2L, 2L, 1L), (row.VertexCount, row.TriangleCount, row.EdgeLinkCount, row.DoorLinkCount));
        }

        [Fact]
        public void ExteriorMesh_GridMatchesTheOwningCellsGrid_NotTheMutagenSwappedPair()
        {
            var row = Row(NavMeshFixture.Linked);
            var cell = db.QuerySingle<CellRow>("SELECT * FROM cells WHERE form_key = @key", new { key = row.Cell });

            Assert.Equal((TinyPlugin.ExteriorGridX, TinyPlugin.ExteriorGridY), (row.GridX, row.GridY));
            Assert.Equal((cell.GridX, cell.GridY), (row.GridX, row.GridY));
        }

        [Fact]
        public void Vertices_AreRawFloatTriplesInFileOrder()
        {
            Assert.Equal(
                Bytes(4096f, -8192f, 10f, 4200f, -8192f, 12f, 4096f, -8100f, 14f, 4200f, -8100f, 16.5f),
                Row(NavMeshFixture.Linked).Vertices);
        }

        [Fact]
        public void Triangles_KeepRawNeighbourValuesAndFlags()
        {
            // Edge-flagged neighbours stay edge-link indices, unflagged ones stay triangle indices or -1.
            Assert.Equal(
                Bytes((ushort)0, (ushort)1, (ushort)2, (short)-1, (short)1, (short)0, (ushort)0x0844,
                      (ushort)1, (ushort)3, (ushort)2, (short)1, (short)-1, (short)0, (ushort)0x0C01),
                Row(NavMeshFixture.Linked).Triangles);
        }

        [Fact]
        public void EdgeLinks_StoreTypeTargetEdgeTargetTriangleAndTargetFormId()
        {
            uint neighbour = IdOf(NavMeshFixture.Neighbour);

            Assert.Equal(
                Bytes((byte)2, (byte)1, (ushort)5, neighbour, (byte)0, (byte)2, (ushort)0, neighbour),
                Row(NavMeshFixture.Linked).EdgeLinks);
        }

        [Fact]
        public void EdgeLinkTargets_ResolveThroughTheFormIdColumn()
        {
            uint target = BitConverter.ToUInt32(Row(NavMeshFixture.Linked).EdgeLinks, 4);

            Assert.Equal(db.KeyOf(NavMeshFixture.Neighbour),
                db.QuerySingle<string>("SELECT form_key FROM navigation_meshes WHERE form_id = @id", new { id = (long)target }));
        }

        [Fact]
        public void DoorLinks_StoreTriangleAndDoorRefFormId()
        {
            Assert.Equal(Bytes((ushort)1, IdOf("ExteriorDoor")), Row(NavMeshFixture.Linked).DoorLinks);
        }

        [Fact]
        public void LinkToATargetWithNoRow_IsStoredRawWithItsFormId()
        {
            var row = Row(NavMeshFixture.DanglingLink);

            Assert.Equal(Bytes((byte)0, (byte)0, (ushort)0, NavMeshFixture.DanglingTargetId), row.EdgeLinks);
            Assert.Equal(0, db.QuerySingle<long>("SELECT COUNT(*) FROM navigation_meshes WHERE form_id = @id", new { id = (long)NavMeshFixture.DanglingTargetId }));
        }

        [Fact]
        public void MeshWithoutLinks_HasEmptyLinkBlobsAndZeroCounts()
        {
            var row = Row(NavMeshFixture.Neighbour);

            Assert.Equal((0L, 0L), (row.EdgeLinkCount, row.DoorLinkCount));
            Assert.Empty(row.EdgeLinks);
            Assert.Empty(row.DoorLinks);
        }

        [Fact]
        public void InteriorMesh_HasOwnerCellAndNoWorldspaceOrGrid_AndRecordFlagsStayRaw()
        {
            var row = Row(NavMeshFixture.Interior);

            Assert.Equal(db.KeyOf(TinyPlugin.InteriorCellEditorId), row.Cell);
            Assert.Null(row.Worldspace);
            Assert.Null(row.GridX);
            Assert.Null(row.GridY);
            Assert.Equal((long)unchecked((uint)NavMeshFixture.InteriorRecordFlags), row.RecordFlags);
            Assert.Equal(Bytes(-100f, -200f, 50f, 100f, -200f, 50f, 0f, 300f, 60f), row.Vertices);
            Assert.Equal(Bytes((ushort)0, IdOf("InteriorDoor")), row.DoorLinks);
        }

        [Fact]
        public void BlobLengths_MatchTheCountColumnsTimesTheStrides()
        {
            var rows = db.Query<NavMeshRecord>("SELECT * FROM navigation_meshes");

            Assert.All(rows, row =>
            {
                Assert.Equal(row.VertexCount * 12, row.Vertices.Length);
                Assert.Equal(row.TriangleCount * 14, row.Triangles.Length);
                Assert.Equal(row.EdgeLinkCount * 8, row.EdgeLinks.Length);
                Assert.Equal(row.DoorLinkCount * 6, row.DoorLinks.Length);
            });
        }

        [Fact]
        public void InvalidMeshes_AreSkippedFromTheTableAndFromTheRecordsTable()
        {
            string[] invalid = [db.KeyOf(NavMeshFixture.BadVertexIndex), db.KeyOf(NavMeshFixture.WrongGrid)];

            Assert.Empty(db.Query<string>("SELECT form_key FROM navigation_meshes WHERE form_key IN @invalid", new { invalid }));
            Assert.Empty(db.Query<string>("SELECT form_key FROM records WHERE form_key IN @invalid", new { invalid }));
        }

        [Fact]
        public void EveryImportedMesh_HasOneNavmRecordsRow_AndNoLegacyLabel()
        {
            var tableKeys = db.Query<string>("SELECT form_key FROM navigation_meshes ORDER BY form_key");
            var recordKeys = db.Query<string>("SELECT form_key FROM records WHERE record_type = 'NAVM' ORDER BY form_key");

            Assert.Equal(NavMeshFixture.ImportedCount, tableKeys.Count);
            Assert.Equal(tableKeys, recordKeys);
            Assert.Equal(0, db.QuerySingle<long>("SELECT COUNT(*) FROM records WHERE record_type = 'NVNM'"));
        }
    }

    // NAVMs that violate the contract are logged with their form key and skipped; legal oddities are stored raw.
    public class NavMeshEncoderTests
    {
        static NavMeshBuilder MeshWithThreeVertices()
        {
            var plugin = new TinyPlugin();
            plugin.AddWorldspace("World");
            return plugin.AddExteriorNavMesh("Mesh")
                .Vertex(0f, 0f, 0f).Vertex(1f, 0f, 0f).Vertex(0f, 1f, 0f);
        }

        static EncodedNavMesh Encode(NavMeshBuilder builder) => NavMeshEncoder.Encode(builder.Mesh);

        static void AssertRejected(NavMeshBuilder builder, string reason) =>
            Assert.Contains(reason, Assert.Throws<NavMeshValidationException>(() => Encode(builder)).Message);

        [Fact]
        public void VertexIndexEqualToVertexCount_IsRejected() =>
            AssertRejected(MeshWithThreeVertices().Triangle(0, 1, 3, -1, -1, -1, 0), "vertex 2 index 3 outside 3 vertices");

        [Fact]
        public void NegativeVertexIndex_IsRejected() =>
            AssertRejected(MeshWithThreeVertices().Triangle(-1, 1, 2, -1, -1, -1, 0), "vertex 0 index -1");

        [Fact]
        public void LastValidVertexIndex_IsAccepted() =>
            Assert.Equal(1, Encode(MeshWithThreeVertices().Triangle(0, 1, 2, -1, -1, -1, 0)).TriangleCount);

        [Fact]
        public void NeighbourEqualToTriangleCount_IsRejected() =>
            AssertRejected(MeshWithThreeVertices().Triangle(0, 1, 2, 1, -1, -1, 0), "edge 0 neighbour index 1 outside 1");

        [Fact]
        public void NeighbourBelowMinusOne_IsRejected() =>
            AssertRejected(MeshWithThreeVertices().Triangle(0, 1, 2, -2, -1, -1, 0), "neighbour index -2");

        [Fact]
        public void FlaggedEdgeWithLinkIndexEqualToLinkCount_IsRejected() =>
            AssertRejected(
                MeshWithThreeVertices().Triangle(0, 1, 2, -1, -1, 1, NavMeshBuilder.ExternalEdge2).EdgeLink(0, null, 0, 0),
                "edge 2 edge-link index 1 outside 1");

        [Fact]
        public void FlaggedEdgeWithNegativeLinkIndex_IsRejected() =>
            AssertRejected(
                MeshWithThreeVertices().Triangle(0, 1, 2, -1, -1, -1, NavMeshBuilder.ExternalEdge2).EdgeLink(0, null, 0, 0),
                "edge 2 edge-link index -1");

        [Fact]
        public void FlaggedEdgeWithLastLinkIndex_IsAccepted() =>
            Assert.Equal(1, Encode(
                MeshWithThreeVertices().Triangle(0, 1, 2, -1, -1, 0, NavMeshBuilder.ExternalEdge2).EdgeLink(0, null, 0, 0)).EdgeLinkCount);

        [Fact]
        public void FlaggedEdgeIndexAboveTheTriangleCountButInsideTheLinks_IsAccepted() =>
            Assert.Equal(2, Encode(
                MeshWithThreeVertices().Triangle(0, 1, 2, -1, -1, 1, NavMeshBuilder.ExternalEdge2).EdgeLink(0, null, 0, 0).EdgeLink(0, null, 0, 0)).EdgeLinkCount);

        [Theory]
        [InlineData(256)]
        [InlineData(-1)]
        public void LinkTypeOutsideAByte_IsRejected(int type) =>
            AssertRejected(MeshWithThreeVertices().EdgeLink(type, null, 0, 0), "outside 0..255");

        [Fact]
        public void LinkTypeOfTheLargestByte_IsStoredRaw() =>
            Assert.Equal(255, Encode(MeshWithThreeVertices().EdgeLink(255, null, 0, 0)).EdgeLinks[0]);

        [Fact]
        public void NegativeTargetTriangleOrEdge_IsRejected()
        {
            AssertRejected(MeshWithThreeVertices().EdgeLink(0, null, -1, 0), "target triangle -1");
            AssertRejected(MeshWithThreeVertices().EdgeLink(0, null, 0, -1), "target edge -1");
        }

        static NavMeshBuilder MeshWithOneTriangle() =>
            MeshWithThreeVertices().Triangle(0, 1, 2, -1, -1, -1, 0);

        [Fact]
        public void NegativeDoorTriangle_IsRejected() =>
            AssertRejected(MeshWithOneTriangle().DoorLink(-1, null), "door link 0 triangle -1");

        [Fact]
        public void DoorTriangleEqualToTriangleCount_IsRejected() =>
            AssertRejected(MeshWithOneTriangle().DoorLink(1, null), "door link 0 triangle 1 outside 1 triangles");

        [Fact]
        public void DoorTriangleOfTheLastTriangle_IsAccepted() =>
            Assert.Equal(1, Encode(MeshWithOneTriangle().DoorLink(0, null)).DoorLinkCount);

        [Fact]
        public void LinkTargetInAnotherPlugin_IsRejected()
        {
            var foreign = FormKey.Factory("000123:Other.esm");

            AssertRejected(MeshWithThreeVertices().EdgeLink(0, foreign, 0, 0), "belongs to another plugin");
            AssertRejected(MeshWithOneTriangle().DoorLink(0, foreign), "belongs to another plugin");
        }

        [Fact]
        public void LinkWithoutTarget_IsStoredWithFormIdZero()
        {
            var encoded = Encode(MeshWithOneTriangle().EdgeLink(1, null, 3, 2).DoorLink(0, null));

            Assert.Equal(0u, BitConverter.ToUInt32(encoded.EdgeLinks, 4));
            Assert.Equal(0u, BitConverter.ToUInt32(encoded.DoorLinks, 2));
        }

        [Fact]
        public void FlaggedEdge1_IndexesTheEdgeLinks()
        {
            Assert.Equal(1, Encode(
                MeshWithThreeVertices().Triangle(0, 1, 2, -1, 0, -1, NavMeshBuilder.ExternalEdge1).EdgeLink(0, null, 0, 0)).EdgeLinkCount);
            AssertRejected(
                MeshWithThreeVertices().Triangle(0, 1, 2, -1, 1, -1, NavMeshBuilder.ExternalEdge1).EdgeLink(0, null, 0, 0),
                "edge 1 edge-link index 1 outside 1");
        }

        [Fact]
        public void MeshWithoutVerticesAndTriangles_EncodesToEmptyBlobs()
        {
            var plugin = new TinyPlugin();
            plugin.AddWorldspace("World");

            var encoded = Encode(plugin.AddExteriorNavMesh("Empty"));

            Assert.Equal((0, 0, 0, 0), (encoded.VertexCount, encoded.TriangleCount, encoded.EdgeLinkCount, encoded.DoorLinkCount));
            Assert.Empty(encoded.Vertices);
            Assert.Empty(encoded.Triangles);
        }

        [Fact]
        public void MeshWithoutGeometry_IsRejected()
        {
            var plugin = new TinyPlugin();
            plugin.AddWorldspace("World");

            AssertRejected(plugin.AddExteriorNavMesh("NoGeometry").WithoutGeometry(), "no navmesh geometry");
        }

        [Fact]
        public void Blobs_AreLittleEndian()
        {
            var builder = MeshWithOneTriangle().Vertex(4096f, -8192f, 10f);
            var target = new FormKey(builder.Mesh.FormKey.ModKey, 0xABCDEF);
            builder.Triangle(0, 1, 2, 1, -1, 0, 0x0844).EdgeLink(2, target, 0x0105, 1).DoorLink(0, target);

            var encoded = Encode(builder);

            Assert.Equal(new byte[] { 0x00, 0x00, 0x80, 0x45, 0x00, 0x00, 0x00, 0xC6, 0x00, 0x00, 0x20, 0x41 }, encoded.Vertices[^12..]);
            Assert.Equal(new byte[] { 0x00, 0x00, 0x01, 0x00, 0x02, 0x00, 0x01, 0x00, 0xFF, 0xFF, 0x00, 0x00, 0x44, 0x08 }, encoded.Triangles[^14..]);
            Assert.Equal(new byte[] { 0x02, 0x01, 0x05, 0x01, 0xEF, 0xCD, 0xAB, 0x00 }, encoded.EdgeLinks);
            Assert.Equal(new byte[] { 0x00, 0x00, 0xEF, 0xCD, 0xAB, 0x00 }, encoded.DoorLinks);
        }

        [Fact]
        public void LinkNoTriangleEdgePointsAt_IsStoredRaw() =>
            Assert.Equal(2, Encode(MeshWithThreeVertices().Triangle(0, 1, 2, -1, -1, -1, 0).EdgeLink(0, null, 0, 0).EdgeLink(0, null, 0, 0)).EdgeLinkCount);

        [Fact]
        public void ExteriorGrid_IsReadFromTheSwappedMutagenPair()
        {
            var plugin = new TinyPlugin();
            plugin.AddWorldspace("World");

            var parent = Assert.IsType<NavMeshParent.Exterior>(
                Encode(plugin.AddExteriorNavMesh("Mesh", gridX: 7, gridY: -21)).Parent);

            Assert.Equal((7, -21), (parent.GridX, parent.GridY));
        }
    }

    // The owner reconciliation of the cell walk, against in-memory cells.
    public class NavMeshOwnerTests
    {
        // Lets a single bad mesh among a handful of good ones through; the production 1% default is pinned by the boundary tests below.
        const double Tolerant = 0.5;

        sealed class Scenario
        {
            public TinyPlugin Plugin { get; } = new();
            public StringWriter Errors { get; } = new();

            public Scenario() => Plugin.AddWorldspace("World");

            public PreparedNavMeshes Prepare(double maxSkippedFraction = PreparedNavMeshes.DefaultMaxSkippedFraction) => PreparedNavMeshes.From(
                [new NavMeshCellScope(Plugin.Exterior, Plugin.Worldspace.FormKey), new NavMeshCellScope(Plugin.Interior, null)],
                Errors,
                maxSkippedFraction);

            public NavMeshBuilder AddValidExterior(string name) => Plugin.AddExteriorNavMesh(name)
                .Vertex(0f, 0f, 0f).Vertex(1f, 0f, 0f).Vertex(0f, 1f, 0f).Triangle(0, 1, 2, -1, -1, -1, 0);

            public void AddValidExteriors(int count)
            {
                for (int i = 0; i < count; i++) AddValidExterior($"Good{i}");
            }

            public void AddInvalidExteriors(int count)
            {
                for (int i = 0; i < count; i++)
                    Plugin.AddExteriorNavMesh($"Bad{i}").Vertex(0f, 0f, 0f).Triangle(0, 1, 2, -1, -1, -1, 0);
            }
        }

        [Fact]
        public void MeshWithGridOfAnotherCell_IsSkippedAndReportedWithItsFormKey()
        {
            var scenario = new Scenario();
            scenario.AddValidExterior("Good");
            var mesh = scenario.Plugin.AddExteriorNavMesh("Off", gridX: 9, gridY: 9)
                .Vertex(0f, 0f, 0f).Vertex(1f, 0f, 0f).Vertex(0f, 1f, 0f).Triangle(0, 1, 2, -1, -1, -1, 0);

            var prepared = scenario.Prepare(Tolerant);

            Assert.Single(prepared.Entries);
            Assert.Equal(1, prepared.Skipped);
            Assert.Contains($"NAVM ERROR {mesh.Mesh.FormKey}", scenario.Errors.ToString());
            Assert.Contains("differs from its cell's grid", scenario.Errors.ToString());
        }

        [Fact]
        public void InteriorMeshNamingAnotherParentCell_IsSkipped()
        {
            var scenario = new Scenario();
            scenario.AddValidExterior("Good");
            scenario.Plugin.AddInteriorNavMesh("Stray", parentCell: scenario.Plugin.Exterior.FormKey)
                .Vertex(0f, 0f, 0f).Vertex(1f, 0f, 0f).Vertex(0f, 1f, 0f).Triangle(0, 1, 2, -1, -1, -1, 0);

            scenario.Prepare(Tolerant);

            Assert.Contains("differs from its owning cell", scenario.Errors.ToString());
        }

        [Fact]
        public void ExteriorMeshDeclaringAnotherWorldspace_IsSkipped()
        {
            var scenario = new Scenario();
            scenario.AddValidExterior("Good");
            scenario.Plugin.AddExteriorNavMesh("Elsewhere", declaredWorldspace: FormKey.Factory("000123:Fixture.esm"))
                .Vertex(0f, 0f, 0f).Vertex(1f, 0f, 0f).Vertex(0f, 1f, 0f).Triangle(0, 1, 2, -1, -1, -1, 0);

            var prepared = scenario.Prepare(Tolerant);

            Assert.Equal(1, prepared.Skipped);
            Assert.Contains("mesh worldspace 000123:Fixture.esm differs from its cell's worldspace", scenario.Errors.ToString());
        }

        [Fact]
        public void ExteriorMeshWithoutWorldspace_IsSkipped()
        {
            var scenario = new Scenario();
            scenario.AddValidExterior("Good");
            scenario.Plugin.AddExteriorNavMesh("Orphan", declaredWorldspace: FormKey.Null)
                .Vertex(0f, 0f, 0f).Vertex(1f, 0f, 0f).Vertex(0f, 1f, 0f).Triangle(0, 1, 2, -1, -1, -1, 0);

            var prepared = scenario.Prepare(Tolerant);

            Assert.Equal(1, prepared.Skipped);
            Assert.Contains("mesh worldspace (none)", scenario.Errors.ToString());
        }

        [Fact]
        public void ExteriorMeshInACellOutsideAnyWorldspace_FailsTheExtraction()
        {
            var scenario = new Scenario();
            scenario.AddValidExteriors(3);

            Assert.Throws<NavMeshExtractionException>(
                () => PreparedNavMeshes.From([new NavMeshCellScope(scenario.Plugin.Exterior, null)], scenario.Errors));
            Assert.Contains("differs from its cell's worldspace (none)", scenario.Errors.ToString());
        }

        [Fact]
        public void ExteriorMeshInACellWithoutGrid_IsSkipped()
        {
            var scenario = new Scenario();
            scenario.AddValidExterior("Good");
            scenario.Plugin.AddExteriorNavMeshInInteriorCell("NoGrid")
                .Vertex(0f, 0f, 0f).Vertex(1f, 0f, 0f).Vertex(0f, 1f, 0f).Triangle(0, 1, 2, -1, -1, -1, 0);

            var prepared = scenario.Prepare(Tolerant);

            Assert.Equal(1, prepared.Skipped);
            Assert.Contains("cell without a grid position", scenario.Errors.ToString());
        }

        [Fact]
        public void LiveMeshWithoutGeometry_IsSkipped()
        {
            var scenario = new Scenario();
            scenario.AddValidExterior("Good");
            scenario.Plugin.AddExteriorNavMesh("Hollow").WithoutGeometry();

            var prepared = scenario.Prepare(Tolerant);

            Assert.Equal(1, prepared.Skipped);
            Assert.Contains("no navmesh geometry", scenario.Errors.ToString());
        }

        [Fact]
        public void EachEntry_PairsTheRecordWithTheRowOfItsOwnCell()
        {
            var scenario = new Scenario();
            var good = scenario.AddValidExterior("Good");
            var interior = scenario.Plugin.AddInteriorNavMesh("Inside")
                .Vertex(0f, 0f, 0f).Vertex(1f, 0f, 0f).Vertex(0f, 1f, 0f).Triangle(0, 1, 2, -1, -1, -1, 0);

            var prepared = scenario.Prepare(Tolerant);

            Assert.Equal(
                [(good.Mesh.FormKey.ToString(), scenario.Plugin.Exterior.FormKey.ToString()),
                 (interior.Mesh.FormKey.ToString(), scenario.Plugin.Interior.FormKey.ToString())],
                prepared.Entries.Select(entry => (entry.Record.FormKey.ToString(), entry.Row.Cell)));
        }

        [Fact]
        public void DeletedMesh_IsLeftOutWithoutAnError()
        {
            var scenario = new Scenario();
            scenario.Plugin.AddExteriorNavMesh("Gone").RecordFlags(0x20);

            var prepared = scenario.Prepare();

            Assert.Empty(prepared.Entries);
            Assert.Equal(0, prepared.Skipped);
            Assert.Empty(scenario.Errors.ToString());
        }

        [Fact]
        public void IsolatedFailures_AtTheToleratedRate_KeepTheValidMeshes()
        {
            var scenario = new Scenario();
            scenario.AddValidExteriors(99);
            scenario.AddInvalidExteriors(1);

            var prepared = scenario.Prepare();

            Assert.Equal(99, prepared.Entries.Count);
            Assert.Equal(1, prepared.Skipped);
        }

        [Fact]
        public void FailuresAboveTheToleratedRate_FailTheExtraction()
        {
            var scenario = new Scenario();
            scenario.AddValidExteriors(98);
            scenario.AddInvalidExteriors(2);

            var failure = Assert.Throws<NavMeshExtractionException>(() => scenario.Prepare());

            Assert.Contains("2 of 100 navigation meshes failed validation", failure.Message);
        }

        [Fact]
        public void LiveMeshesButNoRowsAtAll_FailTheExtraction()
        {
            var scenario = new Scenario();
            scenario.AddInvalidExteriors(1);

            Assert.Throws<NavMeshExtractionException>(() => scenario.Prepare());
        }
    }

    [Collection(FixtureEsm.Name)]
    public sealed class NavMeshTableReloadTests : IDisposable
    {
        readonly LoadedFixtureDatabase _fixture;
        readonly TempDirectory _directory = new();
        readonly string _dbPath;

        public NavMeshTableReloadTests(LoadedFixtureDatabase fixture)
        {
            _fixture = fixture;
            _dbPath = Path.Combine(_directory.Path, "data.db");
            File.Copy(fixture.DatabasePath, _dbPath);
        }

        public void Dispose()
        {
            SqliteConnection.ClearAllPools();
            _directory.Dispose();
        }

        SqliteConnection Open()
        {
            var connection = new SqliteConnection(new SqliteConnectionStringBuilder { DataSource = _dbPath, Pooling = false }.ToString());
            connection.Open();
            return connection;
        }

        T Scalar<T>(string sql, object? parameters = null)
        {
            using var connection = Open();
            return connection.QuerySingle<T>(sql, parameters);
        }

        void Execute(string sql)
        {
            using var connection = Open();
            connection.Execute(sql);
        }

        Task Reload() =>
            NavMeshFixture.Loader(_dbPath).ReloadTablesAsync(_fixture.EsmPath, TableReloadPlan.Create(["navigation_meshes"]), TestContext.Current.CancellationToken);

        [Fact]
        public async Task ReloadingOnlyTheTable_RebuildsEveryRowAndItsRecordsRowFromTheEsm()
        {
            string linked = _fixture.KeyOf(NavMeshFixture.Linked);
            byte[] expectedTriangles = Scalar<byte[]>("SELECT triangles FROM navigation_meshes WHERE form_key = @linked", new { linked });
            Execute("DELETE FROM navigation_meshes");
            Execute("DELETE FROM records WHERE record_type = 'NAVM'");
            Execute("INSERT INTO records (form_key, record_type, editor_id, source_plugin) VALUES ('0FFFFF:Fixture.esm', 'NVNM', NULL, 'Fixture.esm')");
            Execute("INSERT INTO navigation_meshes (form_key, vertex_count) VALUES ('0FFFFF:Fixture.esm', 99)");

            await Reload();

            Assert.Equal(NavMeshFixture.ImportedCount, Scalar<long>("SELECT COUNT(*) FROM navigation_meshes"));
            Assert.Equal(NavMeshFixture.ImportedCount, Scalar<long>("SELECT COUNT(*) FROM records WHERE record_type = 'NAVM'"));
            Assert.Equal(0, Scalar<long>("SELECT COUNT(*) FROM records WHERE record_type = 'NVNM'"));
            Assert.Equal(expectedTriangles, Scalar<byte[]>("SELECT triangles FROM navigation_meshes WHERE form_key = @linked", new { linked }));
        }

        [Fact]
        public async Task ReloadingWithASystemicFailure_ThrowsAndLeavesThePreviousTableIntact()
        {
            long meshes = Scalar<long>("SELECT COUNT(*) FROM navigation_meshes");
            long records = Scalar<long>("SELECT COUNT(*) FROM records WHERE record_type = 'NAVM'");
            var strict = new ESMLoader(_dbPath) { Memory = MemoryBudget.Unlimited(), NavMeshMaxSkippedFraction = 0 };

            await Assert.ThrowsAsync<NavMeshExtractionException>(() =>
                strict.ReloadTablesAsync(_fixture.EsmPath, TableReloadPlan.Create(["navigation_meshes"]), TestContext.Current.CancellationToken));

            Assert.Equal(NavMeshFixture.ImportedCount, meshes);
            Assert.Equal((meshes, records), (Scalar<long>("SELECT COUNT(*) FROM navigation_meshes"), Scalar<long>("SELECT COUNT(*) FROM records WHERE record_type = 'NAVM'")));
        }

        [Fact]
        public async Task LoadingTheEsmWithASystemicNavMeshFailure_AbortsTheLoad()
        {
            var strict = new ESMLoader(Path.Combine(_directory.Path, "fresh.db")) { Memory = MemoryBudget.Unlimited(), NavMeshMaxSkippedFraction = 0 };

            await Assert.ThrowsAsync<NavMeshExtractionException>(() =>
                strict.LoadESMAsync(_fixture.EsmPath, ct: TestContext.Current.CancellationToken));
        }

        [Fact]
        public async Task ReloadingOnlyTheTable_LeavesCellsAndPlacedObjectsAlone()
        {
            long cells = Scalar<long>("SELECT COUNT(*) FROM cells");
            long placed = Scalar<long>("SELECT COUNT(*) FROM placed_objects");

            await Reload();

            Assert.Equal((cells, placed), (Scalar<long>("SELECT COUNT(*) FROM cells"), Scalar<long>("SELECT COUNT(*) FROM placed_objects")));
        }
    }

    [Collection(FixtureEsm.Name)]
    public sealed class NavMeshTableMigrationTests(LoadedFixtureDatabase fixture) : IDisposable
    {
        readonly TempDirectory _directory = new();

        public void Dispose()
        {
            SqliteConnection.ClearAllPools();
            _directory.Dispose();
        }

        string CreateLegacyDatabase()
        {
            string path = Path.Combine(_directory.Path, "data.db");
            using (var legacy = new SqliteConnection($"Data Source={path}"))
            {
                legacy.Open();
                legacy.Execute("CREATE TABLE navigation_meshes (form_key TEXT PRIMARY KEY, editor_id TEXT)");
                legacy.Execute("INSERT INTO navigation_meshes VALUES ('000001:A.esm', 'Old')");
                legacy.Execute("INSERT INTO navigation_meshes VALUES ('000002:A.esm', 'Older')");
            }
            SqliteConnection.ClearAllPools();
            return path;
        }

        [Fact]
        public async Task Initialize_DatabaseWithTheTrackingOnlyStub_AddsTheColumnsAndIndexesAndKeepsItsRows()
        {
            using var manager = new DatabaseManager(CreateLegacyDatabase());

            await manager.InitializeAsync(bulkLoad: false);

            var columns = manager.Connection.Query<string>("SELECT name FROM pragma_table_info('navigation_meshes')").ToHashSet();
            Assert.Superset(new HashSet<string> { "form_id", "cell", "worldspace", "grid_x", "grid_y", "record_flags", "format_version",
                "vertex_count", "triangle_count", "edge_link_count", "door_link_count", "vertices", "triangles", "edge_links", "door_links" }, columns);
            var indexes = manager.Connection.Query<string>("SELECT name FROM pragma_index_list('navigation_meshes')").ToHashSet();
            Assert.Superset(new HashSet<string> { "idx_navmesh_ws_grid", "idx_navmesh_cell", "idx_navmesh_form_id" }, indexes);
            Assert.Equal(2, manager.Connection.ExecuteScalar<long>("SELECT COUNT(*) FROM navigation_meshes WHERE editor_id IS NOT NULL AND format_version IS NULL"));
        }

        [Fact]
        public async Task FormIdIndex_RejectsADuplicateId_ButAllowsTheNullIdsOfMigratedStubRows()
        {
            using var manager = new DatabaseManager(CreateLegacyDatabase());
            await manager.InitializeAsync(bulkLoad: false);

            Assert.Equal(2, manager.Connection.ExecuteScalar<long>("SELECT COUNT(*) FROM navigation_meshes WHERE form_id IS NULL"));
            manager.Connection.Execute("INSERT INTO navigation_meshes (form_key, form_id) VALUES ('000010:A.esm', 16)");

            Assert.Throws<SqliteException>(() =>
                manager.Connection.Execute("INSERT INTO navigation_meshes (form_key, form_id) VALUES ('000011:A.esm', 16)"));
        }

        [Fact]
        public async Task Reload_OfAMigratedDatabaseWithNvnmRecordsRows_ReplacesTheStubRowsAndTheLegacyLabel()
        {
            string path = CreateLegacyDatabase();
            using (var manager = new DatabaseManager(path))
            {
                await manager.InitializeAsync(bulkLoad: false);
                manager.Connection.Execute("INSERT INTO records (form_key, record_type, editor_id, source_plugin) VALUES ('000001:A.esm', 'NVNM', 'Old', 'A.esm')");
                manager.Connection.Execute("INSERT INTO records (form_key, record_type, editor_id, source_plugin) VALUES ('000002:A.esm', 'NVNM', 'Older', 'A.esm')");
            }
            SqliteConnection.ClearAllPools();

            await NavMeshFixture.Loader(path).ReloadTablesAsync(fixture.EsmPath, TableReloadPlan.Create(["navigation_meshes"]), TestContext.Current.CancellationToken);

            using var connection = new SqliteConnection(new SqliteConnectionStringBuilder { DataSource = path, Pooling = false }.ToString());
            connection.Open();
            Assert.Equal(NavMeshFixture.ImportedCount, connection.ExecuteScalar<long>("SELECT COUNT(*) FROM navigation_meshes WHERE format_version = 15"));
            Assert.Equal(0, connection.ExecuteScalar<long>("SELECT COUNT(*) FROM navigation_meshes WHERE format_version IS NULL"));
            Assert.Equal(0, connection.ExecuteScalar<long>("SELECT COUNT(*) FROM records WHERE record_type = 'NVNM'"));
            Assert.Equal(NavMeshFixture.ImportedCount, connection.ExecuteScalar<long>("SELECT COUNT(*) FROM records WHERE record_type = 'NAVM'"));
        }
    }
}
