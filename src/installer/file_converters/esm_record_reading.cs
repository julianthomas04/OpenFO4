using Mutagen.Bethesda.Fallout4;

namespace AssetConverter
{
    // How the load and the work discovery read the same record groups and cell batches, so both see the same records.
    public partial class ESMLoader
    {
        private static bool IsUnknownRecordType(Exception ex) =>
            ex.Message.Contains("Unknown major record type") ||
            (ex.InnerException?.Message.Contains("Unknown major record type") ?? false);

        // Reads a record group; a record type Mutagen does not know is not a Fallout 4 record type and yields skipped.
        private static TResult ReadGroup<T, TResult>(IEnumerable<T> records, Func<IEnumerable<T>, TResult> read, TResult skipped)
        {
            try { return read(records); }
            catch (Exception ex) when (IsUnknownRecordType(ex)) { return skipped; }
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
    }
}
