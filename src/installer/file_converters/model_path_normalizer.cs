using Microsoft.Data.Sqlite;

namespace AssetConverter
{
    /// <summary>
    /// Install step 3b: lowercases every <c>model_path</c> / <c>lod_model_path</c> column in the database so the
    /// engine can look the deployed GLBs up case-insensitively on any filesystem.
    /// </summary>
    public static class ModelPathNormalizer
    {
        static readonly string[] PathColumns = ["model_path", "lod_model_path"];

        /// <param name="onlyTables">When set, only these tables are normalized (partial reloads).</param>
        public static int Run(string dbPath, IReadOnlyCollection<string>? onlyTables = null)
        {
            using var conn = new SqliteConnection(new SqliteConnectionStringBuilder { DataSource = dbPath, Pooling = false }.ToString());
            conn.Open();

            int total = 0;
            foreach (var table in ListTables(conn))
            {
                if (onlyTables != null && !onlyTables.Contains(table)) continue;
                foreach (var column in ListColumns(conn, table).Where(PathColumns.Contains))
                {
                    using var update = conn.CreateCommand();
                    update.CommandText = $"UPDATE \"{table}\" SET {column} = LOWER({column}) WHERE {column} IS NOT NULL AND {column} != LOWER({column})";
                    int rows = update.ExecuteNonQuery();
                    if (rows > 0) Console.WriteLine($"[NormalizePaths] {table}.{column}: lowercased {rows} rows");
                    total += rows;
                }
            }
            Console.WriteLine($"[NormalizePaths] {total} model path rows normalized");
            return total;
        }

        static List<string> ListTables(SqliteConnection conn)
        {
            var tables = new List<string>();
            using var cmd = conn.CreateCommand();
            cmd.CommandText = "SELECT name FROM sqlite_master WHERE type = 'table'";
            using var reader = cmd.ExecuteReader();
            while (reader.Read()) tables.Add(reader.GetString(0));
            return tables;
        }

        static List<string> ListColumns(SqliteConnection conn, string table)
        {
            var columns = new List<string>();
            using var info = conn.CreateCommand();
            info.CommandText = $"PRAGMA table_info(\"{table}\")";
            using var reader = info.ExecuteReader();
            while (reader.Read()) columns.Add(reader.GetString(1));
            return columns;
        }
    }
}
