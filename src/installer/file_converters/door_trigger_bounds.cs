using System.Text.Json;
using Dapper;
using Microsoft.Data.Sqlite;
using SharpBSABA2;
using SharpBSABA2.BA2Util;

namespace AssetConverter
{
    /// <summary>
    /// Fills <c>doors.trigger_bounds</c> for the automatic (walk-in) doors: their OBND is empty, the volume that triggers them is the
    /// editor-marker geometry of their model. Runs after the ESM load, which writes the doors. An automatic door left without them fails the
    /// step, since walking into it would load nothing.
    /// </summary>
    internal static class DoorTriggerBounds
    {
        /// <summary>DOOR record flag "Automatic" (the door loads its destination when touched).</summary>
        public const int AutomaticFlag = 0x02;

        private sealed record DoorRow(string FormKey, string? EditorId, string? ModelPath, string? Flags);

        /// <param name="readMesh">The bytes of a mesh by its path below the game's meshes root as the DOOR record names it, or null when the game has none.</param>
        /// <returns>The number of automatic doors that received trigger bounds, which is all of them.</returns>
        /// <exception cref="DoorTriggerBoundsException">An automatic door has no trigger bounds; nothing is written.</exception>
        public static int Run(string dbPath, Func<string, byte[]?> readMesh)
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
                    failures.Add($"{subject}: {failure}");
                    continue;
                }
                conn.Execute("UPDATE doors SET trigger_bounds = @Json WHERE form_key = @FormKey", new { Json = json, door.FormKey }, transaction);
                filled++;
            }
            if (failures.Count > 0) throw new DoorTriggerBoundsException(failures);
            transaction.Commit();

            Console.WriteLine($"[DoorTriggerBounds] {filled} of {automatic.Count} automatic doors have trigger bounds");
            return filled;
        }

        /// <summary>Runs against the meshes in the archives of <paramref name="gameDataDirectory"/>.</summary>
        /// <exception cref="FileNotFoundException">A mesh archive is missing.</exception>
        /// <exception cref="DoorTriggerBoundsException">An automatic door has no trigger bounds; nothing is written.</exception>
        public static int RunOnGame(string dbPath, string gameDataDirectory)
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
            catch (InvalidDataException ex)
            {
                return (null, $"its model '{model}' could not be read: {ex.Message}");
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
            var paths = ArchiveNames.Select(name => Path.Combine(gameDataDirectory, name)).ToList();
            foreach (string missing in paths.Where(path => !File.Exists(path)))
                throw new FileNotFoundException($"{missing} not found; the doors' trigger bounds cannot be read from their models.", missing);
            try
            {
                foreach (string path in paths)
                {
                    var entries = new Dictionary<string, ArchiveEntry>(StringComparer.OrdinalIgnoreCase);
                    var archive = new BA2(path);
                    _archives.Add((archive, entries));
                    foreach (var file in archive.Files) entries[file.FullPath.Replace('/', '\\')] = file;
                }
            }
            catch
            {
                Dispose();
                throw;
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

    /// <summary>Automatic doors whose trigger bounds could not be measured from their models, each named with the reason.</summary>
    internal sealed class DoorTriggerBoundsException(IReadOnlyList<string> failures)
        : ConversionStepException($"{failures.Count} automatic door(s) have no trigger bounds, so walking into them would load nothing: {string.Join("; ", failures)}")
    {
        public IReadOnlyList<string> Failures { get; } = failures;
    }
}
