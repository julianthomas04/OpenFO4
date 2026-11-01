using AssetConverter.Progress;
using Mutagen.Bethesda.Fallout4;

namespace AssetConverter
{
    /// <summary>What the cell load reports: one unit per cell, two per placed object (placed_objects and records), one per teleport and per landscape.</summary>
    internal readonly record struct CellWorkCounts(long Cells, long Placed, long Teleports, long Landscapes)
    {
        public long Units => Cells + 2 * Placed + Teleports + Landscapes;

        public static CellWorkCounts operator +(CellWorkCounts left, CellWorkCounts right) => new(
            left.Cells + right.Cells, left.Placed + right.Placed, left.Teleports + right.Teleports, left.Landscapes + right.Landscapes);
    }

    // Count-only discovery: before the load starts, every unit of work it will tick is counted, so the progress total is
    // known and sealed once. Nothing is kept: the load reads the records again.
    public partial class ESMLoader
    {
        private static bool IsUnknownRecordType(Exception ex) =>
            ex.Message.Contains("Unknown major record type") ||
            (ex.InnerException?.Message.Contains("Unknown major record type") ?? false);

        // A record type Mutagen does not know counts as empty, as ProcessGroupAsync skips it.
        internal static long CountGroup<T>(IEnumerable<T> records)
        {
            try { return records.Count(); }
            catch (Exception ex) when (IsUnknownRecordType(ex)) { return 0; }
        }

        private static List<ICellGetter> WorldspaceCells(IWorldspaceGetter worldspace)
        {
            var cells = new List<ICellGetter>();
            if (worldspace.TopCell != null) cells.Add(worldspace.TopCell);
            if (worldspace.SubCells != null) ExtractCellsRecursive(worldspace.SubCells, cells);
            return cells;
        }

        private static List<ICellGetter> TopLevelCells(IFallout4ModGetter file)
        {
            var cells = new List<ICellGetter>();
            if (file.Cells != null) ExtractCellsRecursive(file.Cells, cells);
            return cells;
        }

        internal static CellWorkCounts CountCellBatch(IReadOnlyCollection<ICellGetter> cells)
        {
            var placed = cells.Aggregate(default(PlacedCounts), (sum, cell) => sum + PlacedReferences.Count(cell));
            return new CellWorkCounts(cells.Count, placed.Placed, placed.Teleports, cells.Count(cell => cell.Landscape != null));
        }

        // The cell batches of the load: one per worldspace plus the top-level interiors.
        internal long CountCellWork(IFallout4ModGetter file)
        {
            var total = default(CellWorkCounts);
            foreach (var batch in file.Worldspaces.Select(WorldspaceCells).Append(TopLevelCells(file)))
            {
                _ct.ThrowIfCancellationRequested();
                total += CountCellBatch(batch);
            }
            return total.Units;
        }

        private async Task DiscoverAndSealTotalAsync(IFallout4ModGetter file)
        {
            long total;
            using (EsmTiming.MeasurePhase("discovery.wall"))
            {
                var counts = AllJobs.Where(job => job.CountWork != null)
                    .Select(job => Task.Run(() => job.CountWork!(this, file), _ct));
                total = (await Task.WhenAll(counts)).Sum();
            }
            Console.WriteLine($"Discovery: {total:N0} units of work.");
            InstallProgress.AddTotal(_progressPhase, total);
            InstallProgress.SealTotal(_progressPhase);
        }
    }
}
