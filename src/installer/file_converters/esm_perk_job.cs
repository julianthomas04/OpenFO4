using Dapper;
using Microsoft.Data.Sqlite;
using Mutagen.Bethesda.Fallout4;
using Mutagen.Bethesda.Plugins;

namespace AssetConverter
{
    public partial class ESMLoader
    {
        static readonly ModKey BaseGame = ModKey.FromNameAndExtension("Fallout4.esm");

        private async Task ProcessPerksAsync(IFallout4ModGetter file, SqliteConnection conn, SqliteTransaction? transaction)
        {
            var perks = file.Perks.ToList();
            await ProcessGroupAsync(perks, "PERK", conn, new PerkProcessor(), transaction);
            if (perks.Count == 0) return;

            var chains = PerkRankChains.Build(perks);
            foreach (var warning in chains.Warnings)
                Console.Error.WriteLine($"  WARNING: {warning}");
            var chart = PerkChartBuilder.Build(perks, file.EnumerateMajorRecords<IActorValueInformationGetter>(), chains);
            if (file.ModKey == BaseGame)
                PerkChartBuilder.RequireCompleteChart(chart);

            await WriteFirstRanksAsync(chains, conn, transaction);
            await WritePerkChartAsync(chart, conn, transaction);
            await FillChartPerkMoviesAsync(chart, conn, transaction);
            Console.WriteLine($"Perk chart: {chart.Count} first-rank perks in {PerkChartBuilder.CoveredCells(chart)} of {PerkChartBuilder.ColumnCount * PerkChartBuilder.RowCount(chart)} cells.");
        }

        private static Task WriteFirstRanksAsync(PerkRankChains chains, SqliteConnection conn, SqliteTransaction? transaction)
        {
            const string sql = "UPDATE perks SET first_rank = @FirstRank WHERE form_key = @Perk";
            var rows = chains.FirstRankOf.Select(pair => new { Perk = pair.Key.ToString(), FirstRank = pair.Value.ToString() });
            return RunInTransactionAsync(conn, transaction, tx => conn.ExecuteAsync(sql, rows, tx));
        }

        private static Task WritePerkChartAsync(IReadOnlyList<PerkChartEntry> chart, SqliteConnection conn, SqliteTransaction? transaction)
        {
            const string sql = "INSERT OR REPLACE INTO perk_chart (perk, special, required_special) VALUES (@Perk, @Special, @RequiredSpecial)";
            var rows = chart.Select(entry => new { Perk = entry.Perk.ToString(), entry.Special, entry.RequiredSpecial });
            return RunInTransactionAsync(conn, transaction, tx => conn.ExecuteAsync(sql, rows, tx));
        }

        private static Task FillChartPerkMoviesAsync(IReadOnlyList<PerkChartEntry> chart, SqliteConnection conn, SqliteTransaction? transaction)
        {
            const string sql = "UPDATE perks SET swf_path = @SwfPath WHERE form_key = @Perk AND (swf_path IS NULL OR swf_path = '')";
            var rows = chart.Select(entry => new { Perk = entry.Perk.ToString(), SwfPath = PipboyArtCatalog.PerkMoviePath(entry.Perk.ID) });
            return RunInTransactionAsync(conn, transaction, tx => conn.ExecuteAsync(sql, rows, tx));
        }

        private static async Task RunInTransactionAsync(SqliteConnection conn, SqliteTransaction? transaction, Func<SqliteTransaction, Task> work)
        {
            bool ownTransaction = transaction is null;
            transaction ??= conn.BeginTransaction();
            try
            {
                await work(transaction);
                if (ownTransaction) transaction.Commit();
            }
            finally
            {
                if (ownTransaction) transaction.Dispose();
            }
        }
    }
}
