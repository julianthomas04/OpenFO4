using Dapper;
using Microsoft.Data.Sqlite;
using Mutagen.Bethesda.Fallout4;

namespace AssetConverter
{
    public sealed class NavMeshProcessor : BaseRecordProcessor<INavigationMeshGetter>
    {
        const string InsertSql = @"
            INSERT OR REPLACE INTO navigation_meshes (
                form_key, editor_id, form_id, cell, worldspace, grid_x, grid_y, record_flags, format_version,
                vertex_count, triangle_count, edge_link_count, door_link_count,
                vertices, triangles, edge_links, door_links)
            VALUES (
                @FormKey, @EditorId, @FormId, @Cell, @Worldspace, @GridX, @GridY, @RecordFlags, @FormatVersion,
                @VertexCount, @TriangleCount, @EdgeLinkCount, @DoorLinkCount,
                @Vertices, @Triangles, @EdgeLinks, @DoorLinks)";

        readonly Dictionary<INavigationMeshGetter, NavMeshRow> _rows = new(ReferenceEqualityComparer.Instance);

        internal NavMeshProcessor(IEnumerable<(INavigationMeshGetter Record, NavMeshRow Row)> entries)
        {
            foreach (var (record, row) in entries)
                _rows.Add(record, row);
        }

        protected override async Task ProcessSpecificRecordAsync(INavigationMeshGetter record, string formKey, SqliteConnection conn, SqliteTransaction transaction)
        {
            var row = _rows[record];
            var mesh = row.Mesh;
            await conn.ExecuteAsync(InsertSql, new
            {
                FormKey = formKey,
                EditorId = record.EditorID,
                FormId = (long)mesh.FormId,
                row.Cell,
                row.Worldspace,
                row.GridX,
                row.GridY,
                RecordFlags = (long)mesh.RecordFlags,
                FormatVersion = (long)mesh.FormatVersion,
                mesh.VertexCount,
                mesh.TriangleCount,
                mesh.EdgeLinkCount,
                mesh.DoorLinkCount,
                mesh.Vertices,
                mesh.Triangles,
                mesh.EdgeLinks,
                mesh.DoorLinks,
            }, transaction);
        }
    }

    public partial class ESMLoader
    {
        // NVNM is the navmesh subrecord; records rows of older databases were mislabelled with it instead of NAVM.
        internal const string LegacyNavMeshRecordType = "NVNM";

        internal double NavMeshMaxSkippedFraction { get; init; } = PreparedNavMeshes.DefaultMaxSkippedFraction;

        private static RecordJob NavMeshJob() => new(
            ["navigation_meshes"], ["NAVM", LegacyNavMeshRecordType],
            (loader, context, conn, transaction) => loader.ProcessNavMeshesAsync(context.File, conn, transaction));

        private async Task ProcessNavMeshesAsync(IFallout4ModGetter file, SqliteConnection conn, SqliteTransaction? transaction)
        {
            var prepared = PreparedNavMeshes.From(EnumerateCellScopes(file), Console.Error, NavMeshMaxSkippedFraction);
            if (prepared.Skipped > 0)
                Console.WriteLine($"  {prepared.Skipped} navigation meshes skipped as invalid (see NAVM ERROR lines).");
            await ProcessGroupAsync(prepared.Entries.Select(entry => entry.Record).ToList(), "NAVM", conn, new NavMeshProcessor(prepared.Entries), transaction);
        }

        // The same cell walk as the cells table: worldspace cells carry their worldspace, top-level cells have none.
        private static IEnumerable<NavMeshCellScope> EnumerateCellScopes(IFallout4ModGetter file)
        {
            foreach (var worldspace in file.Worldspaces)
            {
                var cells = new List<ICellGetter>();
                if (worldspace.TopCell != null) cells.Add(worldspace.TopCell);
                if (worldspace.SubCells != null) ExtractCellsRecursive(worldspace.SubCells, cells);
                foreach (var cell in cells)
                    yield return new NavMeshCellScope(cell, worldspace.FormKey);
            }

            var topCells = new List<ICellGetter>();
            if (file.Cells != null) ExtractCellsRecursive(file.Cells, topCells);
            foreach (var cell in topCells)
                yield return new NavMeshCellScope(cell, null);
        }
    }
}
