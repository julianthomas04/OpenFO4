using Dapper;
using Microsoft.Data.Sqlite;
using Mutagen.Bethesda.Fallout4;

namespace AssetConverter
{
    public partial class ESMLoader
    {
        private async Task ProcessActorValuesAsync(IFallout4ModGetter file, SqliteConnection conn, SqliteTransaction? transaction)
        {
            var actorValues = file.EnumerateMajorRecords<IActorValueInformationGetter>().ToList();
            await ProcessGroupAsync(actorValues, "AVIF", conn, new SimpleRecordProcessor<IActorValueInformationGetter>("actor_value_informations"), transaction);

            var attributes = SpecialAttributes.Collect(actorValues);
            if (attributes.Count == 0) return;
            const string sql = "INSERT OR REPLACE INTO special_attributes (special, form_key, name, description) VALUES (@Special, @FormKey, @Name, @Description)";
            var rows = attributes.Select(entry => new { entry.Special, FormKey = entry.Record.ToString(), entry.Name, entry.Description });
            await RunInTransactionAsync(conn, transaction, tx => conn.ExecuteAsync(sql, rows, tx));
            Console.WriteLine($"SPECIAL attributes: {attributes.Count} stored.");
        }
    }
}
