namespace AssetConverter
{
    /// <summary>Command line of the dev subcommand <c>esm-only [--tables &lt;table&gt;[,&lt;table&gt;...]]</c>.</summary>
    internal static class EsmOnlyArguments
    {
        const string TablesFlag = "--tables";
        const string TablesFlagWithValue = "--tables=";

        /// <summary>
        /// Parses the full argument list (<c>args[0]</c> is the subcommand). Returns false for a malformed command
        /// line; otherwise <paramref name="tablesCsv"/> is the raw table list, or null for a full database rebuild.
        /// </summary>
        public static bool TryParse(string[] args, out string? tablesCsv)
        {
            tablesCsv = null;
            if (args.Length <= 1) return true;
            if (args.Length == 3 && args[1] == TablesFlag)
            {
                tablesCsv = args[2];
                return true;
            }
            if (args.Length == 2 && args[1].StartsWith(TablesFlagWithValue, StringComparison.Ordinal))
            {
                tablesCsv = args[1][TablesFlagWithValue.Length..];
                return true;
            }
            return false;
        }

        public static string[] SplitTables(string tablesCsv) =>
            tablesCsv.Split(',', StringSplitOptions.RemoveEmptyEntries | StringSplitOptions.TrimEntries);
    }
}
