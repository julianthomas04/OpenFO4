using AssetConverter;
using Microsoft.Data.Sqlite;
using Mutagen.Bethesda.Fallout4;
using Xunit;

namespace InstallerTests
{
    public sealed class BatchInsertTests : IDisposable
    {
        readonly SqliteConnection _connection = new("Data Source=:memory:");

        public BatchInsertTests()
        {
            _connection.Open();
            Execute("CREATE TABLE t (id INTEGER PRIMARY KEY, name TEXT, payload BLOB)");
        }

        public void Dispose() => _connection.Dispose();

        void Execute(string sql)
        {
            using var command = _connection.CreateCommand();
            command.CommandText = sql;
            command.ExecuteNonQuery();
        }

        long Scalar(string sql)
        {
            using var command = _connection.CreateCommand();
            command.CommandText = sql;
            return (long)command.ExecuteScalar()!;
        }

        static List<object?[]> Rows(int count) =>
            Enumerable.Range(1, count).Select(i => new object?[] { i, $"row{i}", null }).ToList();

        static readonly string[] Columns = ["id", "name", "payload"];

        [Theory]
        [InlineData(1)]
        [InlineData(999)]
        [InlineData(1000)]
        [InlineData(1001)]
        [InlineData(2503)]
        public void Execute_InsertsEveryRow(int rowCount)
        {
            BatchInsert.Execute(_connection, null, "t", Columns, Rows(rowCount));

            Assert.Equal(rowCount, Scalar("SELECT COUNT(*) FROM t"));
            Assert.Equal(rowCount, Scalar("SELECT MAX(id) FROM t"));
        }

        [Fact]
        public void Execute_KeepsInsertionOrderAsRowid()
        {
            Execute("CREATE TABLE keyed (key TEXT PRIMARY KEY, name TEXT)");
            string[] keys = ["e", "c", "i", "a", "g"];

            BatchInsert.Execute(_connection, null, "keyed", ["key", "name"], keys.Select(key => new object?[] { key, null }).ToList());

            using var command = _connection.CreateCommand();
            command.CommandText = "SELECT key FROM keyed ORDER BY rowid";
            using var reader = command.ExecuteReader();
            var inserted = new List<string>();
            while (reader.Read()) inserted.Add(reader.GetString(0));
            Assert.Equal(keys, inserted);
        }

        [Fact]
        public void Execute_NoRows_IsANoOp()
        {
            BatchInsert.Execute(_connection, null, "t", Columns, []);

            Assert.Equal(0, Scalar("SELECT COUNT(*) FROM t"));
        }

        [Fact]
        public void Execute_NullValue_IsStoredAsSqlNull()
        {
            BatchInsert.Execute(_connection, null, "t", Columns, [[1, null, null]]);

            Assert.Equal(1, Scalar("SELECT COUNT(*) FROM t WHERE name IS NULL AND payload IS NULL"));
        }

        [Fact]
        public void Execute_ByteArray_RoundTripsAsBlob()
        {
            BatchInsert.Execute(_connection, null, "t", Columns, [[1, "x", new byte[] { 0, 1, 254, 255 }]]);

            using var command = _connection.CreateCommand();
            command.CommandText = "SELECT payload FROM t";
            Assert.Equal(new byte[] { 0, 1, 254, 255 }, (byte[])command.ExecuteScalar()!);
        }

        [Fact]
        public void Execute_DuplicatePrimaryKey_ReplacesTheExistingRow()
        {
            BatchInsert.Execute(_connection, null, "t", Columns, [[1, "old", null]]);

            BatchInsert.Execute(_connection, null, "t", Columns, [[1, "new", null]]);

            Assert.Equal(1, Scalar("SELECT COUNT(*) FROM t"));
            Assert.Equal(1, Scalar("SELECT COUNT(*) FROM t WHERE name = 'new'"));
        }

        [Fact]
        public void Execute_InsideTransaction_IsRolledBackWithIt()
        {
            using (var transaction = _connection.BeginTransaction())
            {
                BatchInsert.Execute(_connection, transaction, "t", Columns, Rows(600));
                transaction.Rollback();
            }

            Assert.Equal(0, Scalar("SELECT COUNT(*) FROM t"));
        }

        [Fact]
        public void Execute_WithoutTransaction_LeavesNoTransactionOpen()
        {
            BatchInsert.Execute(_connection, null, "t", Columns, Rows(10));

            Assert.Equal(10, Scalar("SELECT COUNT(*) FROM t"));
            using var transaction = _connection.BeginTransaction();
        }

        [Theory]
        [InlineData(2)]
        [InlineData(4)]
        public void Execute_RowWithWrongValueCount_ThrowsBeforeInsertingAnyRow(int badWidth)
        {
            var rows = new List<object?[]> { new object?[] { 1, "a", null }, new object?[badWidth] };

            Assert.Throws<ArgumentException>(() => BatchInsert.Execute(_connection, null, "t", Columns, rows));

            Assert.Equal(0, Scalar("SELECT COUNT(*) FROM t"));
        }

        [Fact]
        public void Execute_RowWithWrongValueCountInsideACallerTransaction_LeavesNothingForTheCallerToCommit()
        {
            var rows = new List<object?[]> { new object?[] { 1, "a", null }, new object?[] { 2, "b" } };

            using (var transaction = _connection.BeginTransaction())
            {
                Assert.Throws<ArgumentException>(() => BatchInsert.Execute(_connection, transaction, "t", Columns, rows));
                transaction.Commit();
            }

            Assert.Equal(0, Scalar("SELECT COUNT(*) FROM t"));
        }

        [Fact]
        public void Execute_InsideTransaction_PersistsWhenTheCallerCommits()
        {
            using (var transaction = _connection.BeginTransaction())
            {
                BatchInsert.Execute(_connection, transaction, "t", Columns, Rows(1500));
                transaction.Commit();
            }

            Assert.Equal(1500, Scalar("SELECT COUNT(*) FROM t"));
        }

        [Fact]
        public void Execute_InsertFailsMidwayWithoutCallerTransaction_RollsBackEveryRowAndLeavesTheConnectionUsable()
        {
            Execute("CREATE TABLE strict_t (id INTEGER PRIMARY KEY, name TEXT NOT NULL)");
            var rows = new List<object?[]> { new object?[] { 1, "a" }, new object?[] { 2, "b" }, new object?[] { 3, null } };

            Assert.Throws<SqliteException>(() => BatchInsert.Execute(_connection, null, "strict_t", ["id", "name"], rows));

            Assert.Equal(0, Scalar("SELECT COUNT(*) FROM strict_t"));
            using var transaction = _connection.BeginTransaction();
        }

        [Fact]
        public void Execute_InsertFailsMidwayInsideCallerTransaction_LeavesRollbackToTheCaller()
        {
            Execute("CREATE TABLE strict_t (id INTEGER PRIMARY KEY, name TEXT NOT NULL)");
            var rows = new List<object?[]> { new object?[] { 1, "a" }, new object?[] { 2, null } };

            using var transaction = _connection.BeginTransaction();
            Assert.Throws<SqliteException>(() => BatchInsert.Execute(_connection, transaction, "strict_t", ["id", "name"], rows));

            Assert.Equal(1, Scalar("SELECT COUNT(*) FROM strict_t"));
            transaction.Rollback();
            Assert.Equal(0, Scalar("SELECT COUNT(*) FROM strict_t"));
        }

        [Fact]
        public void Execute_TenOrMoreColumns_BindsEveryColumnToItsOwnValue()
        {
            string[] columns = [.. Enumerable.Range(0, 12).Select(c => $"c{c}")];
            Execute($"CREATE TABLE wide ({string.Join(", ", columns.Select(c => c + " INTEGER"))})");
            var row = Enumerable.Range(0, 12).Select(c => (object?)(c * 100 + 1)).ToArray();

            BatchInsert.Execute(_connection, null, "wide", columns, [row]);

            using var command = _connection.CreateCommand();
            command.CommandText = $"SELECT {string.Join(", ", columns)} FROM wide";
            using var reader = command.ExecuteReader();
            Assert.True(reader.Read());
            Assert.Equal(Enumerable.Range(0, 12).Select(c => c * 100L + 1), Enumerable.Range(0, 12).Select(reader.GetInt64));
        }

        // Columns without a declared type keep the storage class SQLite is given, so this pins what each CLR type is stored as.
        [Fact]
        public void Execute_ClrValues_AreStoredWithTheirNaturalStorageClass()
        {
            Execute("CREATE TABLE untyped (id INTEGER PRIMARY KEY, v)");
            object?[] values = [1.25, 0.1f, true, false, long.MaxValue, 7, DayOfWeek.Friday, "text", new byte[] { 1, 2, 3 }, null];
            var rows = values.Select((value, i) => new object?[] { i, value }).ToList();

            BatchInsert.Execute(_connection, null, "untyped", ["id", "v"], rows);

            using var command = _connection.CreateCommand();
            command.CommandText = "SELECT typeof(v), v FROM untyped ORDER BY id";
            using var reader = command.ExecuteReader();
            var stored = new List<(string Type, object? Value)>();
            while (reader.Read()) stored.Add((reader.GetString(0), reader.IsDBNull(1) ? null : reader.GetValue(1)));

            Assert.Equal(
                new (string, object?)[]
                {
                    ("real", 1.25), ("real", (double)0.1f), ("integer", 1L), ("integer", 0L), ("integer", long.MaxValue),
                    ("integer", 7L), ("integer", 5L), ("text", "text"), ("blob", new byte[] { 1, 2, 3 }), ("null", null),
                },
                stored);
        }

        [Theory]
        [InlineData(1, new[] { 1 })]
        [InlineData(999, new[] { 999 })]
        [InlineData(1000, new[] { 1000 })]
        [InlineData(2503, new[] { 1000, 1000, 503 })]
        public void Execute_ReportsInsertedRowsInProgressSizedSteps(int rowCount, int[] expectedSteps)
        {
            var steps = new List<int>();

            BatchInsert.Execute(_connection, null, "t", Columns, Rows(rowCount), steps.Add);

            Assert.Equal(expectedSteps, steps);
        }
    }

    // A failed ROLLBACK must never replace the error that made the caller roll back.
    public sealed class TransactionRollbackTests : IDisposable
    {
        readonly SqliteConnection _connection = new("Data Source=:memory:");

        public TransactionRollbackTests()
        {
            _connection.Open();
            Execute("CREATE TABLE t (id INTEGER PRIMARY KEY)");
        }

        public void Dispose() => _connection.Dispose();

        void Execute(string sql)
        {
            using var command = _connection.CreateCommand();
            command.CommandText = sql;
            command.ExecuteNonQuery();
        }

        long Count()
        {
            using var command = _connection.CreateCommand();
            command.CommandText = "SELECT COUNT(*) FROM t";
            return (long)command.ExecuteScalar()!;
        }

        [Fact]
        public void AfterFailure_OpenTransaction_IsRolledBackSilently()
        {
            var log = new StringWriter();
            using var transaction = _connection.BeginTransaction();
            Execute("INSERT INTO t VALUES (1)");

            TransactionRollback.AfterFailure(transaction, log);

            Assert.Equal(0, Count());
            Assert.Equal("", log.ToString());
        }

        [Fact]
        public void AfterFailure_TransactionSqliteAlreadyRolledBack_NeitherThrowsNorReports()
        {
            var log = new StringWriter();
            using var transaction = _connection.BeginTransaction();
            Execute("INSERT INTO t VALUES (1)");
            Execute("ROLLBACK");

            TransactionRollback.AfterFailure(transaction, log);

            Assert.Equal("", log.ToString());
        }

        [Fact]
        public void AfterFailure_RollbackThatThrows_IsReportedInsteadOfPropagated()
        {
            var log = new StringWriter();
            var transaction = _connection.BeginTransaction();
            _connection.Close();

            TransactionRollback.AfterFailure(transaction, log);

            Assert.Contains("Rollback after a failed transaction also failed", log.ToString());
            Assert.Contains(nameof(InvalidOperationException), log.ToString());
        }
    }

    // MaterialSwapProcessor.ParseAlphaToken is the inverse of MaterialCensus.GetAlphaToken: the engine reads the
    // numeric columns instead of re-parsing the token grammar ("blend|<mult>", "test:<cutoff>", empty = opaque).
    public class AlphaTokenTests
    {
        [Theory]
        [InlineData("", false, 1.0, null)]
        [InlineData("blend", true, 1.0, null)]
        [InlineData("blend|0.50", true, 0.5, null)]
        [InlineData("blend|2", true, 2.0, null)]
        [InlineData("test:128", false, 1.0, 128)]
        [InlineData("test:0", false, 1.0, 0)]
        public void ParseAlphaToken_KnownTokens_ProduceTheDocumentedColumns(string token, bool blend, double mult, int? test)
        {
            MaterialSwapProcessor.ParseAlphaToken(token, out bool isBlend, out double alphaMult, out int? alphaTest);

            Assert.Equal((blend, mult, test), (isBlend, alphaMult, alphaTest));
        }

        [Theory]
        [InlineData("blend|abc")]
        [InlineData("blend|")]
        public void ParseAlphaToken_BlendWithUnparsableMultiplier_KeepsDefaultMultiplier(string token)
        {
            MaterialSwapProcessor.ParseAlphaToken(token, out bool isBlend, out double alphaMult, out _);

            Assert.True(isBlend);
            Assert.Equal(1.0, alphaMult);
        }

        [Theory]
        [InlineData("test:abc")]
        [InlineData("test:")]
        [InlineData("garbage")]
        [InlineData("BLEND")]
        public void ParseAlphaToken_UnrecognisedToken_MeansOpaque(string token)
        {
            MaterialSwapProcessor.ParseAlphaToken(token, out bool isBlend, out double alphaMult, out int? alphaTest);

            Assert.Equal((false, 1.0, (int?)null), (isBlend, alphaMult, alphaTest));
        }

        [Fact]
        public void ParseAlphaToken_MultiplierIsInvariantCultureDecimalPoint()
        {
            var previous = System.Globalization.CultureInfo.CurrentCulture;
            System.Globalization.CultureInfo.CurrentCulture = new System.Globalization.CultureInfo("de-DE");
            try
            {
                MaterialSwapProcessor.ParseAlphaToken("blend|0.25", out _, out double alphaMult, out _);

                Assert.Equal(0.25, alphaMult);
            }
            finally
            {
                System.Globalization.CultureInfo.CurrentCulture = previous;
            }
        }
    }

    // BaseRecordProcessor<T>.SafeSerialize: absent data is NULL, a broken column is reported and stored as NULL
    // (never as an empty "{}"), anything that is not a serialisation failure is not swallowed.
    public class SafeSerializeTests
    {
        sealed class Probe : BaseRecordProcessor<IStaticGetter>
        {
            public string? Serialise(object? value) => SafeSerialize(value);
        }

        sealed class Node
        {
            public string? Name { get; init; }
            public int? Missing { get; init; }
            public Node? Self { get; set; }
        }

#pragma warning disable CA1822 // System.Text.Json only serialises instance members
        sealed class ThrowsInvalidOperation { public int Value => throw new InvalidOperationException("broken getter"); }

        sealed class ThrowsOther { public int Value => throw new TimeoutException("bug"); }
#pragma warning restore CA1822

        [Fact]
        public void SafeSerialize_Null_IsNull() =>
            Assert.Null(new Probe().Serialise(null));

        [Fact]
        public void SafeSerialize_NullProperties_AreOmitted() =>
            Assert.Equal("""{"Name":"a"}""", new Probe().Serialise(new Node { Name = "a" }));

        [Fact]
        public void SafeSerialize_ReferenceCycle_IsCutInsteadOfFailing()
        {
            var node = new Node { Name = "loop" };
            node.Self = node;

            Assert.Equal("""{"Name":"loop"}""", new Probe().Serialise(node));
        }

        [Fact]
        public void SafeSerialize_GetterThrowingInvalidOperation_YieldsNullNotEmptyObject() =>
            Assert.Null(new Probe().Serialise(new ThrowsInvalidOperation()));

        [Fact]
        public void SafeSerialize_UnrelatedExceptionType_Propagates() =>
            Assert.Throws<TimeoutException>(() => new Probe().Serialise(new ThrowsOther()));
    }
}
