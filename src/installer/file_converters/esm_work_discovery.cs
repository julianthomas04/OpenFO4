using System.Runtime.ExceptionServices;
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
        // Enumerated rather than counted by Count(): a collection-backed group may know its size without ever reading the
        // records, while the load reads them and fails on a record type Mutagen does not know.
        internal static long CountGroup<T>(IEnumerable<T> records) => ReadGroup(records, CountEnumerated, skipped: 0L);

        private static long CountEnumerated<T>(IEnumerable<T> records)
        {
            long count = 0;
            foreach (var _ in records) count++;
            return count;
        }

        internal static CellWorkCounts CountCellBatch(IReadOnlyCollection<ICellGetter> cells)
        {
            var placed = cells.Aggregate(default(PlacedCounts), (sum, cell) => sum + PlacedReferences.Count(cell));
            return new CellWorkCounts(cells.Count, placed.Placed, placed.Teleports, cells.Count(cell => cell.Landscape != null));
        }

        // The cell batches of the load: one per worldspace plus the top-level interiors.
        internal static CellWorkCounts CountCells(IFallout4ModGetter file, CancellationToken ct)
        {
            var total = default(CellWorkCounts);
            foreach (var batch in file.Worldspaces.Select(WorldspaceCells).Append(TopLevelCells(file)))
            {
                ct.ThrowIfCancellationRequested();
                total += CountCellBatch(batch);
            }
            return total;
        }

        private async Task DiscoverAndSealTotalAsync(IFallout4ModGetter file)
        {
            long total;
            using (EsmTiming.MeasurePhase("discovery.wall"))
                total = await CountPlannedWorkAsync(file);
            Console.WriteLine($"Discovery: {total:N0} units of work.");
            InstallProgress.AddTotal(_progressPhase, total);
            InstallProgress.SealTotal(_progressPhase);
        }

        // The first job to fail cancels the others, and its own exception is the one thrown, not a sibling's cancellation.
        internal async Task<long> CountPlannedWorkAsync(IFallout4ModGetter file)
        {
            using var cancelOnFault = CancellationTokenSource.CreateLinkedTokenSource(_ct);
            var counts = AllJobs.Where(job => job.CountWork != null).Select(job => Task.Run(() =>
            {
                try { return job.CountWork!(this, file, cancelOnFault.Token); }
                catch { cancelOnFault.Cancel(); throw; }
            }, cancelOnFault.Token)).ToList();

            try
            {
                return (await Task.WhenAll(counts)).Sum();
            }
            catch
            {
                var cause = counts.Where(count => count.IsFaulted).SelectMany(count => count.Exception!.InnerExceptions)
                    .FirstOrDefault(ex => ex is not OperationCanceledException);
                if (cause == null) throw;
                ExceptionDispatchInfo.Capture(cause).Throw();
                throw;
            }
        }
    }
}
