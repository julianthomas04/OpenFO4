using Dapper;
using Microsoft.Data.Sqlite;
using Mutagen.Bethesda.Fallout4;
using Mutagen.Bethesda.Plugins;
using Noggog;

namespace AssetConverter
{
    internal static class LinkKey
    {
        /// <summary>The form key of a link as the tables store it; NULL for a null link.</summary>
        public static string? Text(IFormLinkGetter link) => link.IsNull ? null : link.FormKey.ToString();
    }

    internal sealed record ConditionRow(
        int Function, int CompareOperator, bool OrFlag, int Flags, double? CompareValue, string? CompareGlobal, int RunOn, string? Reference,
        int Param1Number, string? Param1Record, string? Param1String,
        int Param2Number, string? Param2Record, string? Param2String,
        int Param3);

    /// <summary>One record condition as database columns; shapes with no column (non-function data, no comparison value) are an error, never dropped.</summary>
    internal static class ConditionMapper
    {
        public static ConditionRow Map(IConditionGetter condition, string subject)
        {
            if (condition.Data is not IFunctionConditionDataGetter data)
                throw new UnsupportedEsmDataException($"{subject}: a {condition.Data?.GetType().Name ?? "missing"} condition is not a function condition, which is the only shape the database stores.");

            (double? value, string? global) = condition switch
            {
                IConditionFloatGetter literal => ((double?)literal.ComparisonValue, (string?)null),
                IConditionGlobalGetter byGlobal when !byGlobal.ComparisonValue.IsNull => ((double?)null, byGlobal.ComparisonValue.FormKey.ToString()),
                _ => throw new UnsupportedEsmDataException($"{subject}: a {condition.GetType().Name} condition has no comparison value the database can store."),
            };

            return new ConditionRow(
                (int)data.Function, (int)condition.CompareOperator, condition.Flags.HasFlag(Condition.Flag.OR), (int)(condition.Flags & ~Condition.Flag.OR), value, global,
                (int)data.RunOnType, LinkKey.Text(data.Reference),
                data.ParameterOneNumber, LinkKey.Text(data.ParameterOneRecord), data.ParameterOneString,
                data.ParameterTwoNumber, LinkKey.Text(data.ParameterTwoRecord), data.ParameterTwoString,
                data.Unknown3);
        }
    }

    /// <summary>Writes the conditions of one record into a condition table whose owner column is named by the caller.</summary>
    internal static class ConditionTable
    {
        public static Task InsertAsync(SqliteConnection conn, SqliteTransaction transaction, string table, string ownerColumn,
            string ownerKey, IReadOnlyList<IConditionGetter>? conditions, string subject)
        {
            var rows = (conditions ?? []).Select((condition, ord) => (Ord: ord, Row: ConditionMapper.Map(condition, $"{subject} condition {ord}")))
                .Select(entry => new
                {
                    Owner = ownerKey,
                    entry.Ord,
                    entry.Row.Function,
                    entry.Row.CompareOperator,
                    entry.Row.OrFlag,
                    entry.Row.Flags,
                    entry.Row.CompareValue,
                    entry.Row.CompareGlobal,
                    entry.Row.RunOn,
                    entry.Row.Reference,
                    entry.Row.Param1Number,
                    entry.Row.Param1Record,
                    entry.Row.Param1String,
                    entry.Row.Param2Number,
                    entry.Row.Param2Record,
                    entry.Row.Param2String,
                    entry.Row.Param3,
                }).ToList();
            return conn.ExecuteAsync(
                $@"INSERT OR REPLACE INTO {table}
                    ({ownerColumn}, ord, function, compare_op, or_flag, flags, compare_value, compare_global, run_on, reference,
                     param1_num, param1_record, param1_string, param2_num, param2_record, param2_string, param3)
                  VALUES (@Owner, @Ord, @Function, @CompareOperator, @OrFlag, @Flags, @CompareValue, @CompareGlobal, @RunOn, @Reference,
                          @Param1Number, @Param1Record, @Param1String, @Param2Number, @Param2Record, @Param2String, @Param3)",
                rows, transaction);
        }
    }
}
