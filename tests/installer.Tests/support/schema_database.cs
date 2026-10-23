using AssetConverter;
using Dapper;
using Microsoft.Data.Sqlite;

namespace InstallerTests
{
    /// <summary>A temp SQLite database created through the loader's own <see cref="DatabaseManager"/>, i.e. from the real esm_schema.sql.</summary>
    internal sealed class SchemaDatabase : IDisposable
    {
        readonly TempDirectory _directory = new();
        readonly DatabaseManager _manager;

        SchemaDatabase()
        {
            DatabasePath = Path.Combine(_directory.Path, "data.db");
            _manager = new DatabaseManager(DatabasePath);
        }

        public string DatabasePath { get; }

        public string DirectoryPath => _directory.Path;

        public SqliteConnection Connection => _manager.Connection;

        public static async Task<SchemaDatabase> CreateAsync()
        {
            var database = new SchemaDatabase();
            await database._manager.InitializeAsync(bulkLoad: false);
            await database.Connection.ExecuteAsync("PRAGMA foreign_keys = OFF");
            return database;
        }

        public int Execute(string sql, object? parameters = null) => Connection.Execute(sql, parameters);

        public IReadOnlyList<T> Query<T>(string sql, object? parameters = null) => Connection.Query<T>(sql, parameters).ToList();

        public T QuerySingle<T>(string sql, object? parameters = null) => Connection.QuerySingle<T>(sql, parameters);

        public void Dispose()
        {
            _manager.Dispose();
            SqliteConnection.ClearAllPools();
            _directory.Dispose();
        }
    }
}
