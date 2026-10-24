using System.Text.Json;
using Dapper;
using Microsoft.Data.Sqlite;
using SharpBSABA2;
using SharpBSABA2.BA2Util;

namespace AssetConverter
{
    /// <summary>
    /// Fills <c>doors.trigger_bounds</c> for the automatic (walk-in) doors: their OBND is empty, the volume that triggers them is the
    /// editor-marker geometry of their model. Runs after the ESM load, which writes the doors.
    /// </summary>
    internal static class DoorTriggerBounds
    {
        /// <summary>DOOR record flag "Automatic" (the door loads its destination when touched).</summary>
        public const int AutomaticFlag = 0x02;

        public sealed record Report(int Filled, IReadOnlyList<string> Failures);

        private sealed record DoorRow(string FormKey, string? EditorId, string? ModelPath, string? Flags);

        /// <param name="readMesh">The bytes of a mesh by its path below the game's meshes root as the DOOR record names it, or null when the game has none.</param>
        public static Report Run(string dbPath, Func<string, byte[]?> readMesh)
        {
            using var conn = new SqliteConnection(new SqliteConnectionStringBuilder { DataSource = dbPath, Mode = SqliteOpenMode.ReadWrite }.ToString());
            conn.Open();
            var automatic = conn.Query<DoorRow>("SELECT form_key AS FormKey, editor_id AS EditorId, model_path AS ModelPath, flags AS Flags FROM doors")
                .Where(IsAutomatic).ToList();

            var cache = new Dictionary<string, (string? Json, string? Failure)>(StringComparer.OrdinalIgnoreCase);
            var failures = new List<string>();
            int filled = 0;
            using var transaction = conn.BeginTransaction();
            foreach (var door in automatic)
            {
                string subject = $"{door.FormKey} ({door.EditorId})";
                var (json, failure) = door.ModelPath is { Length: > 0 } model
                    ? cache.TryGetValue(model, out var known) ? known : cache[model] = Measure(model, readMesh)
                    : (null, "has no model");
                if (json == null)
                {
                    failures.Add($"{subject}: no trigger bounds, {failure}");
                    continue;
                }
                conn.Execute("UPDATE doors SET trigger_bounds = @Json WHERE form_key = @FormKey", new { Json = json, door.FormKey }, transaction);
                filled++;
            }
            transaction.Commit();

            foreach (string failure in failures)
                Console.Error.WriteLine($"  WARNING: automatic door {failure}");
            Console.WriteLine($"[DoorTriggerBounds] {filled} of {automatic.Count} automatic doors have trigger bounds");
            return new Report(filled, failures);
        }

        /// <summary>Runs against the meshes in the archives of <paramref name="gameDataDirectory"/>.</summary>
        /// <exception cref="FileNotFoundException">A mesh archive is missing.</exception>
        public static Report RunOnGame(string dbPath, string gameDataDirectory)
        {
            using var meshes = new GameMeshReader(gameDataDirectory);
            return Run(dbPath, meshes.Read);
        }

        public static bool IsAutomatic(string? flagsJson) =>
            flagsJson != null && long.TryParse(flagsJson, out long flags) && (flags & AutomaticFlag) != 0;

        private static bool IsAutomatic(DoorRow door) => IsAutomatic(door.Flags);

        private static (string? Json, string? Failure) Measure(string model, Func<string, byte[]?> readMesh)
        {
            byte[]? bytes = readMesh(model);
            if (bytes == null) return (null, $"its model '{model}' is not in the game's meshes");
            try
            {
                return EditorMarkerBounds.Compute(bytes) is { } box
                    ? (JsonSerializer.Serialize(box), null)
                    : (null, $"its model '{model}' has no EditorMarker geometry");
            }
            catch (Exception ex) when (ex is InvalidDataException or InvalidCastException or NullReferenceException or ArgumentException)
            {
                return (null, $"its model '{model}' could not be read: {ex.GetType().Name}: {ex.Message}");
            }
        }
    }

    /// <summary>Reads single meshes out of the game's mesh archives, the later archive overriding the earlier one as extracting both in order would.</summary>
    internal sealed class GameMeshReader : IDisposable
    {
        const string MeshesFolder = "Meshes\\";
        static readonly string[] ArchiveNames = ["Fallout4 - MeshesExtra.ba2", "Fallout4 - Meshes.ba2"];

        private readonly List<(BA2 Archive, Dictionary<string, ArchiveEntry> Entries)> _archives = [];

        /// <exception cref="FileNotFoundException">A mesh archive is missing from the game's Data folder.</exception>
        public GameMeshReader(string gameDataDirectory)
        {
            foreach (string name in ArchiveNames)
            {
                string path = Path.Combine(gameDataDirectory, name);
                if (!File.Exists(path)) throw new FileNotFoundException($"{path} not found; the doors' trigger bounds cannot be read from their models.", path);
                var archive = new BA2(path);
                var entries = new Dictionary<string, ArchiveEntry>(StringComparer.OrdinalIgnoreCase);
                foreach (var file in archive.Files) entries[file.FullPath.Replace('/', '\\')] = file;
                _archives.Add((archive, entries));
            }
        }

        public byte[]? Read(string modelPath)
        {
            string key = MeshesFolder + modelPath.TrimStart('\\', '/').Replace('/', '\\');
            foreach (var (_, entries) in _archives)
            {
                if (!entries.TryGetValue(key, out var entry)) continue;
                using var data = entry.GetDataStream();
                return data.ToArray();
            }
            return null;
        }

        public void Dispose()
        {
            foreach (var (archive, _) in _archives) archive.Close();
        }
    }
}
