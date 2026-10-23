using AssetConverter.Pipeline;
using Dapper;
using Microsoft.Data.Sqlite;
using Mutagen.Bethesda.Fallout4;

namespace AssetConverter
{
    public sealed class TableSelectionException : Exception
    {
        public TableSelectionException(string message) : base(message) { }
    }

    public sealed class MissingDatabaseException : Exception
    {
        public MissingDatabaseException(string message) : base(message) { }
    }

    public sealed class MissingTableException : Exception
    {
        public MissingTableException(string message) : base(message) { }
    }

    /// <summary>
    /// The set of import jobs needed to re-fill a table selection. Tables that
    /// share a processor are inseparable (the processor fills all of them), so
    /// the selection is expanded to the closure of jobs touching a selected table.
    /// </summary>
    public sealed class TableReloadPlan
    {
        internal IReadOnlyList<RecordJob> Jobs { get; }
        public IReadOnlyList<string> Requested { get; }
        public IReadOnlyList<string> Tables { get; }
        public IReadOnlyList<string> RecordTypes { get; }

        public bool NeedsRawScans => Jobs.Any(j => j.NeedsRawScans);
        public bool ReloadsMaterialSwaps => Tables.Contains("material_swaps");

        private TableReloadPlan(IReadOnlyList<string> requested, IReadOnlyList<RecordJob> jobs)
        {
            Requested = requested;
            Jobs = jobs;
            Tables = jobs.SelectMany(j => j.Tables).Distinct().Order(StringComparer.Ordinal).ToList();
            RecordTypes = jobs.SelectMany(j => j.RecordTypes).Distinct().ToList();
        }

        public static IReadOnlyList<string> ValidTables { get; } =
            ESMLoader.AllJobs.SelectMany(j => j.Tables).Distinct().Order(StringComparer.Ordinal).ToList();

        public static TableReloadPlan Create(IReadOnlyCollection<string> requested)
        {
            string valid = string.Join(", ", ValidTables);
            if (requested.Count == 0)
                throw new TableSelectionException($"No tables given. Valid tables: {valid}");

            var unknown = requested.Where(t => !ValidTables.Contains(t)).ToList();
            if (unknown.Count > 0)
                throw new TableSelectionException(
                    $"Unknown table(s): {string.Join(", ", unknown)}. Valid tables: {valid}");

            var tables = new HashSet<string>(requested, StringComparer.Ordinal);
            var picked = new bool[ESMLoader.AllJobs.Count];
            bool grew;
            do
            {
                grew = false;
                for (int i = 0; i < picked.Length; i++)
                {
                    if (picked[i] || !ESMLoader.AllJobs[i].Tables.Any(tables.Contains)) continue;
                    picked[i] = true;
                    tables.UnionWith(ESMLoader.AllJobs[i].Tables);
                    grew = true;
                }
            } while (grew);

            var jobs = ESMLoader.AllJobs.Where((_, i) => picked[i]).ToList();
            return new TableReloadPlan(requested.Distinct().ToList(), jobs);
        }
    }

    public partial class ESMLoader
    {
        /// <summary>
        /// Re-fills only the planned tables of an existing database inside one
        /// transaction: a failure rolls back to the previous contents.
        /// </summary>
        public async Task ReloadTablesAsync(string esmPath, TableReloadPlan plan, CancellationToken ct = default)
        {
            _ct = ct;
            _progressPhase = -1;
            if (!File.Exists(_dbPath))
                throw new MissingDatabaseException($"No database at {_dbPath}; run esm-only without --tables first.");

            var expanded = plan.Tables.Except(plan.Requested).ToList();
            Console.WriteLine($"Reloading tables: {string.Join(", ", plan.Requested)}");
            if (expanded.Count > 0)
                Console.WriteLine($"  Also reloaded because they share a processor: {string.Join(", ", expanded)}");

            using var memory = await Memory.ReserveAsync(MemoryEstimates.EsmImportBytes, ct).ConfigureAwait(false);
            await EsmTiming.RunReportedAsync(() => ReloadPlannedTablesAsync(esmPath, plan)).ConfigureAwait(false);
        }

        private async Task ReloadPlannedTablesAsync(string esmPath, TableReloadPlan plan)
        {
            using var dbManager = new DatabaseManager(_dbPath);
            await dbManager.InitializeAsync(bulkLoad: false);
            var conn = dbManager.Connection;
            await EnsureTablesExistAsync(conn, plan.Tables);

            using var file = OpenPlugin(esmPath);

            Task<RawScans>? rawScansTask = plan.NeedsRawScans
                ? Task.Run(() => ScanRawEsm(esmPath), _ct)
                : null;
            var context = new ImportContext(file, rawScansTask);

            await conn.ExecuteAsync("PRAGMA foreign_keys = OFF");
            using (var transaction = conn.BeginTransaction())
            {
                await ClearTablesAsync(conn, transaction, plan);
                foreach (var job in plan.Jobs)
                {
                    _ct.ThrowIfCancellationRequested();
                    using (EsmTiming.MeasurePhase("reload.job." + job.Label))
                        await job.Run(this, context, conn, transaction);
                }

                if (plan.Tables.Contains("object_modifications"))
                    await WeaponModJson.ResolveAttachPointNamesAsync(conn, transaction);

                if (plan.ReloadsMaterialSwaps)
                {
                    var rawScans = await (rawScansTask
                        ?? throw new InvalidOperationException("Material swap reload requires raw ESM scans that were not scheduled."));
                    InsertBaseMaterialSwaps(conn, transaction, rawScans.BaseSwaps);
                }

                using (EsmTiming.Measure("reload.commit"))
                    transaction.Commit();
            }

            await dbManager.FinalizePartialReloadAsync();
            await ReportRowCountsAsync(conn, plan.Tables);
        }

        private static async Task EnsureTablesExistAsync(SqliteConnection conn, IReadOnlyList<string> tables)
        {
            var existing = (await conn.QueryAsync<string>("SELECT name FROM sqlite_master WHERE type = 'table'")).ToHashSet();
            var missing = tables.Where(t => !existing.Contains(t)).ToList();
            if (missing.Count > 0)
                throw new MissingTableException(
                    $"Database {conn.DataSource} lacks table(s) {string.Join(", ", missing)} even after applying the schema.");
        }

        private static async Task ClearTablesAsync(SqliteConnection conn, SqliteTransaction transaction, TableReloadPlan plan)
        {
            foreach (var table in plan.Tables)
            {
                int rows = await conn.ExecuteAsync($"DELETE FROM {table}", transaction: transaction);
                Console.WriteLine($"  Cleared {table} ({rows} rows)");
            }
            int records = await conn.ExecuteAsync(
                "DELETE FROM records WHERE record_type IN @Types", new { Types = plan.RecordTypes }, transaction);
            Console.WriteLine($"  Cleared {records} records rows ({string.Join(", ", plan.RecordTypes)})");
        }

        private static async Task ReportRowCountsAsync(SqliteConnection conn, IReadOnlyList<string> tables)
        {
            foreach (var table in tables)
                Console.WriteLine($"  {table}: {await conn.ExecuteScalarAsync<long>($"SELECT COUNT(*) FROM {table}")} rows");
        }
    }
}
