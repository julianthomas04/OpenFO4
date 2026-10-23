using Dapper;
using Microsoft.Data.Sqlite;
using SharpBSABA2.BA2Util;

namespace AssetConverter
{
    /// <summary>A terrain LOD folder name does not match any worldspace editor id in the database.</summary>
    public sealed class UnknownLodWorldspaceException : InvalidOperationException
    {
        public UnknownLodWorldspaceException(string message) : base(message) { }
    }

    /// <summary>
    /// Fills <c>lod_water_quads</c> from the level-4 terrain LOD files (<c>.BTR</c>) of the base-game mesh archives (the
    /// DLC worldspaces are not in the database, which is built from Fallout4.esm only). Runs
    /// after the worldspaces table exists, because the terrain folder name (the worldspace editor id) is stored as
    /// the worldspace form_key like every other table does.
    /// </summary>
    public static class LodWaterStage
    {
        static readonly string[] ArchiveNames = ["Fallout4 - Meshes.ba2", "Fallout4 - MeshesExtra.ba2"];

        /// <summary>Scans the mesh archives and replaces the table contents.</summary>
        public static LodWaterReport Run(string dataDirectory, string dbPath)
        {
            RequireDatabase(dbPath);
            var collection = Scan(dataDirectory);
            WriteTable(dbPath, collection);
            return collection.Report;
        }

        /// <summary>
        /// Reads every mesh archive in <paramref name="dataDirectory"/> and merges the level-4 water quads. Needs no database, so the
        /// install runs it while the database is still being built; <see cref="WriteTable"/> stores the result.
        /// </summary>
        public static LodWaterHeights Scan(string dataDirectory, CancellationToken ct = default)
        {
            if (!Directory.Exists(dataDirectory)) throw new DirectoryNotFoundException($"Game data directory not found: {dataDirectory}");

            var archives = ArchiveNames.Select(n => Path.Combine(dataDirectory, n)).ToList();
            foreach (var missing in archives.Where(a => !File.Exists(a)))
                throw new FileNotFoundException($"Mesh archive not found: {missing}", missing);
            Console.WriteLine($"[LodWater] Scanning {string.Join(", ", ArchiveNames)} for level-{LodWaterCollector.WaterLodLevel} terrain LOD water...");

            var opened = new List<BA2>();
            try
            {
                var entries = new List<(string Path, Func<byte[]> ReadBytes)>();
                var seen = new Dictionary<string, string>(StringComparer.OrdinalIgnoreCase);
                foreach (var archivePath in archives)
                {
                    ct.ThrowIfCancellationRequested();
                    var archive = new BA2(archivePath);
                    opened.Add(archive);
                    foreach (var file in archive.Files)
                    {
                        if (!BtrFileName.TryParse(file.FullPath, out var name) || name.Level != LodWaterCollector.WaterLodLevel) continue;
                        if (seen.TryGetValue(file.FullPath, out var firstArchive))
                            throw new InvalidDataException($"{file.FullPath} exists in both {firstArchive} and {Path.GetFileName(archivePath)}; archive override order is not modelled");
                        seen[file.FullPath] = Path.GetFileName(archivePath);
                        var entry = file;
                        Func<byte[]> readBytes = () =>
                        {
                            ct.ThrowIfCancellationRequested();
                            return entry.GetDataStream().ToArray();
                        };
                        entries.Add((file.FullPath, readBytes));
                    }
                }

                var collection = LodWaterCollector.Collect(entries);
                PrintReport(collection.Report);
                return collection;
            }
            finally
            {
                foreach (var archive in opened) archive.Close();
            }
        }

        static void RequireDatabase(string dbPath)
        {
            if (!File.Exists(dbPath)) throw new FileNotFoundException($"No database at {dbPath}; build it first (esm-only).", dbPath);
        }

        /// <summary>Replaces <c>lod_water_quads</c> with <paramref name="collection"/>; needs the worldspaces table to be filled.</summary>
        public static void WriteTable(string dbPath, LodWaterHeights collection)
        {
            RequireDatabase(dbPath);
            using var manager = new DatabaseManager(dbPath);
            manager.InitializeAsync(bulkLoad: false).GetAwaiter().GetResult();
            var connection = manager.Connection;

            var worldspaces = connection.Query<(string FormKey, string? EditorId)>("SELECT form_key, editor_id FROM worldspaces")
                .Where(w => w.EditorId != null)
                .ToDictionary(w => w.EditorId!, w => w.FormKey, StringComparer.OrdinalIgnoreCase);
            var missing = collection.Report.QuadsPerWorld.Keys.Where(w => !worldspaces.ContainsKey(w)).ToList();
            if (missing.Count > 0)
                throw new UnknownLodWorldspaceException(
                    $"Terrain LOD folder(s) without a worldspace of that editor id in {dbPath}: {string.Join(", ", missing)}");

            using var transaction = connection.BeginTransaction();
            connection.Execute("DELETE FROM lod_water_quads", transaction: transaction);
            connection.Execute(
                "INSERT INTO lod_water_quads (worldspace, cell_x, cell_y, height) VALUES (@Worldspace, @CellX, @CellY, @Height)",
                collection.Heights
                    .OrderBy(p => p.Key.World, StringComparer.OrdinalIgnoreCase).ThenBy(p => p.Key.CellX).ThenBy(p => p.Key.CellY)
                    .Select(p => new { Worldspace = worldspaces[p.Key.World], p.Key.CellX, p.Key.CellY, Height = p.Value }),
                transaction);
            transaction.Commit();
        }

        static void PrintReport(LodWaterReport report)
        {
            Console.WriteLine(
                $"[LodWater] {report.FilesRead} L4 files ({report.FilesWithWater} with water), {report.QuadsRead} quads read; " +
                $"duplicate cells: {report.DuplicatesWithinFile} within a file, {report.DuplicatesAcrossFiles} across files (heights identical)");
            foreach (var (world, count) in report.QuadsPerWorld)
                Console.WriteLine($"[LodWater]   {world}: {count} quads");
        }
    }
}
