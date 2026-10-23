using System.Globalization;
using System.Text.RegularExpressions;
using Dapper;
using Microsoft.Data.Sqlite;

namespace AssetConverter
{
    // Reads _LOD.glb files under <assets>/meshes/LOD/ and writes
    // statics.lod_model_path for matching base models. Runs after NIF conversion so the LOD GLBs exist
    // on disk and after ESM load so the statics table is populated.
    //
    // FO4 convention: Meshes\Foo\Bar.nif -> Meshes\LOD\Foo\Bar_LOD.nif
    // Multi-level: Bar_LOD_0.nif (closest) through Bar_LOD_3.nif (farthest).
    // We prefer _LOD_0 > bare _LOD > _LOD_N so the highest-detail LOD wins.
    public static partial class LodBackfill
    {
        // Captures the LOD suffix at the end of the filename (without extension).
        // "Foo_LOD" -> match, group(1) null
        // "Foo_LOD_0" -> match, group(1) "0"
        private static readonly Regex LodSuffix = LodSuffixRegex();

        public static void Run(string dbPath, string meshesOut)
        {
            string lodDir = Path.Combine(meshesOut, "LOD");
            if (!Directory.Exists(lodDir))
            {
                Console.WriteLine($"[LodBackfill] LOD mesh dir missing, skipping: {lodDir}");
                return;
            }
            if (!File.Exists(dbPath))
            {
                Console.WriteLine($"[LodBackfill] DB missing, skipping: {dbPath}");
                return;
            }

            // Two lookup maps built from on-disk LOD GLBs:
            // 1. fsMap: lowercase base nif path -> (priority, stored LOD path)
            //    Matches by directory structure: LOD/Foo/Bar_LOD.glb -> Foo/Bar.nif
            // 2. eidMap: lowercase base filename (no ext) -> (priority, stored LOD path)
            //    Matches by editor_id: LOD/.../SomeName_LOD.glb -> editor_id "SomeName"
            var fsMap = new Dictionary<string, (int priority, string stored)>(StringComparer.Ordinal);
            var eidMap = new Dictionary<string, (int priority, string stored)>(StringComparer.Ordinal);

            int scanned = 0;
            foreach (string full in Directory.EnumerateFiles(lodDir, "*.glb", SearchOption.AllDirectories))
            {
                scanned++;
                string rel = Path.GetRelativePath(lodDir, full);          // "Architecture/Airport/Foo_LOD.glb"
                string stemNoExt = Path.Combine(
                    Path.GetDirectoryName(rel) ?? "",
                    Path.GetFileNameWithoutExtension(rel)).Replace('\\', '/');
                var m = LodSuffix.Match(stemNoExt);
                if (!m.Success) continue;
                string baseRel = stemNoExt.Substring(0, m.Index);         // "Architecture/Airport/Foo"
                int priority = m.Groups[1].Success
                    ? (m.Groups[1].Value == "0" ? 0 : 2 + int.Parse(m.Groups[1].Value, CultureInfo.InvariantCulture))
                    : 1;

                // Stored path: "LOD\Architecture\Airport\Foo_LOD.nif"
                string stored = ("LOD\\" + rel.Replace('/', '\\')).Replace(".glb", ".nif", StringComparison.OrdinalIgnoreCase);

                // Path-based key
                string pathKey = (baseRel + ".nif").ToLowerInvariant();
                if (!fsMap.TryGetValue(pathKey, out var existing) || priority < existing.priority)
                    fsMap[pathKey] = (priority, stored);

                // Editor-id-based key (just the filename without directory)
                string baseName = Path.GetFileName(baseRel).ToLowerInvariant();
                if (!eidMap.TryGetValue(baseName, out var eidExisting) || priority < eidExisting.priority)
                    eidMap[baseName] = (priority, stored);
            }

            Console.WriteLine($"[LodBackfill] scanned {scanned} GLBs, {fsMap.Count} path keys, {eidMap.Count} editor_id keys");

            var connStr = new SqliteConnectionStringBuilder { DataSource = dbPath, Mode = SqliteOpenMode.ReadWrite }.ToString();
            using var conn = new SqliteConnection(connStr);
            conn.Open();

            // Defensive: add the column if a stale schema is in play. Production
            // schema already declares it via esm_schema.sql, but we never want
            // the UPDATE to fail on a DB produced by an older installer build.
            var colNames = conn.Query<dynamic>("PRAGMA table_info(statics)")
                .Select(r => (string)r.name).ToHashSet();
            if (!colNames.Contains("lod_model_path"))
            {
                conn.Execute("ALTER TABLE statics ADD COLUMN lod_model_path TEXT");
                Console.WriteLine("[LodBackfill] added statics.lod_model_path column");
            }

            // Fetch statics whose model_path doesn't already live under LOD\.
            var rows = conn.Query<(string form_key, string model_path, string? editor_id)>(@"
                SELECT form_key, model_path, editor_id FROM statics
                WHERE model_path IS NOT NULL
                  AND substr(lower(model_path), 1, 4) NOT IN ('lod\', 'lod/')
            ").ToList();

            int matchedByPath = 0;
            int matchedByEid = 0;
            using var tx = conn.BeginTransaction();
            using var upd = conn.CreateCommand();
            upd.Transaction = tx;
            upd.CommandText = "UPDATE statics SET lod_model_path = @p WHERE form_key = @k";
            var pParam = upd.CreateParameter(); pParam.ParameterName = "@p"; upd.Parameters.Add(pParam);
            var kParam = upd.CreateParameter(); kParam.ParameterName = "@k"; upd.Parameters.Add(kParam);

            foreach (var (form_key, model_path, editor_id) in rows)
            {
                // Try path-based match first
                string pathKey = model_path.Replace('\\', '/').ToLowerInvariant();
                if (fsMap.TryGetValue(pathKey, out var hit))
                {
                    pParam.Value = hit.stored;
                    kParam.Value = form_key;
                    upd.ExecuteNonQuery();
                    matchedByPath++;
                    continue;
                }

                // Fall back to editor_id match
                if (editor_id != null)
                {
                    string eidKey = editor_id.ToLowerInvariant();
                    // Strip trailing _LOD from editor_id if present
                    if (eidKey.EndsWith("_lod", StringComparison.Ordinal))
                        eidKey = eidKey[..^4];
                    if (eidMap.TryGetValue(eidKey, out var eidHit))
                    {
                        pParam.Value = eidHit.stored;
                        kParam.Value = form_key;
                        upd.ExecuteNonQuery();
                        matchedByEid++;
                    }
                }
            }
            tx.Commit();

            long populated = (long)(conn.ExecuteScalar(
                "SELECT COUNT(*) FROM statics WHERE lod_model_path IS NOT NULL") ?? 0L);
            long covered = (long)(conn.ExecuteScalar(@"
                SELECT COUNT(*) FROM placed_objects po
                JOIN statics s ON po.base_object = s.form_key
                WHERE s.lod_model_path IS NOT NULL") ?? 0L);
            Console.WriteLine($"[LodBackfill] matched={matchedByPath} by path, {matchedByEid} by editor_id, populated={populated} placed_objects_covered={covered}");
        }

        [GeneratedRegex(@"_LOD(?:_(\d+))?$", RegexOptions.IgnoreCase | RegexOptions.CultureInvariant)]
        private static partial Regex LodSuffixRegex();
    }
}
