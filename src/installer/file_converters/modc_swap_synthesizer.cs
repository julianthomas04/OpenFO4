using System.Text.Json;
using Dapper;
using Microsoft.Data.Sqlite;

namespace AssetConverter
{
    // FO4 bases with a MODC color-remapping float but no (or not only) a MSWP
    // swap — large crates, shipping containers, the burnt-car MSTT fleet —
    // tint their model's grayscale-palette materials by selecting a different
    // LUT band. Rather than teach the engine a second mechanism, synthesize
    // ordinary material_swap_entries: read the deployed GLB's "src:" tokens,
    // re-row every palette material at the MODC float, and point the base's
    // base_material_swaps row at a synthetic "MODC:<base>" swap key.
    // Runs on the finished database (the bases come from base_material_swaps), so the ESM load
    // never waits for the converted GLBs and the extracted, palette-baked textures this reads.
    /// <summary>What one MODC synthesis run did, including the work it could not do (unreadable models, failed bakes).</summary>
    internal sealed record ModcSynthesisSummary(
        int Bases, int Synthesized, int EntriesWritten, int BakedVariants, int VariantCombos,
        int WithoutModel, int WithoutPaletteMaterials, int UnreadableModels, int FailedBakes)
    {
        public override string ToString() =>
            $"MODC synthesis: {Synthesized}/{Bases} bases -> {EntriesWritten} entries, {BakedVariants}/{VariantCombos} variant bakes " +
            $"({WithoutModel} without model, {WithoutPaletteMaterials} without palette materials, {UnreadableModels} unreadable models, {FailedBakes} failed bakes).";
    }

    internal static class ModcSwapSynthesizer
    {
        private static readonly int GlbReadParallelism = Math.Clamp(Environment.ProcessorCount, 4, IoParallelism.MaxThreads);

        private sealed record ModcBase(string FormKey, string? SwapFormKey, float ColorRemapIndex);

        /// <summary>
        /// Synthesizes the swaps of every MODC base of the database at <paramref name="dbPath"/> in one transaction: a failure
        /// leaves the database as it was. Run it after the ESM load and before the LOD backfill and the model path normalization.
        /// </summary>
        internal static ModcSynthesisSummary Run(string dbPath, string meshesOut, string materialsRoot, string? texturesOut, CancellationToken ct = default)
        {
            ModcSynthesisSummary? summary = null;
            EsmTiming.RunReportedAsync(async () => summary = await SynthesizeAllAsync(dbPath, meshesOut, materialsRoot, texturesOut, ct))
                .GetAwaiter().GetResult();
            return summary!;
        }

        private static async Task<ModcSynthesisSummary> SynthesizeAllAsync(string dbPath, string meshesOut, string materialsRoot, string? texturesOut, CancellationToken ct)
        {
            using var conn = new SqliteConnection(new SqliteConnectionStringBuilder
            {
                DataSource = dbPath,
                Mode = SqliteOpenMode.ReadWrite,
                Pooling = false,
            }.ToString());
            await conn.OpenAsync(ct);
            using var transaction = conn.BeginTransaction();
            var summary = await RunAsync(conn, transaction, meshesOut, materialsRoot, texturesOut, ct);
            transaction.Commit();
            return summary;
        }

        internal static async Task<ModcSynthesisSummary> RunAsync(SqliteConnection conn, SqliteTransaction? transaction,
            string meshesOut, string materialsRoot, string? texturesOut, CancellationToken ct = default)
        {
            var modcBases = await ReadModcBasesAsync(conn, transaction);
            if (modcBases.Count == 0) return new ModcSynthesisSummary(0, 0, 0, 0, 0, 0, 0, 0, 0);

            using (EsmTiming.Measure("modc.census"))
            {
                MaterialCensus.Build(materialsRoot);
                NifConverter.EnsureTexturePathContext(materialsRoot);
            }

            Dictionary<string, string> modelByBase, lodByBase;
            using (EsmTiming.Measure("modc.queries"))
            {
                modelByBase = await ReadModelPathsAsync(conn, transaction);
                lodByBase = await ReadLodModelPathsAsync(conn, transaction);
            }

            // meshesOut is already the deployed meshes/ root; model paths
            // are relative to it ("Vehicles\Automotive\Sedan03_Postwar.nif").
            string GlbPath(string model) => Path.Combine(meshesOut, GlbName(model));
            var (sourceKeysByGlb, unreadableGlbs) = ReadSourceKeysOfAll(
                modcBases.Where(b => modelByBase.ContainsKey(b.FormKey))
                    .SelectMany(b => lodByBase.TryGetValue(b.FormKey, out var lod)
                        ? new[] { GlbPath(modelByBase[b.FormKey]), GlbPath(lod) }
                        : new[] { GlbPath(modelByBase[b.FormKey]) }),
                ct);

            int synthesized = 0, entriesWritten = 0, noModel = 0, noPaletteMaterial = 0;
            var lateCombos = new HashSet<(string Diffuse, string Lut, float Row)>();

            using (EsmTiming.Measure("modc.bases"))
            {
                foreach (var (formKey, swapKey, modcIndex) in modcBases)
                {
                    ct.ThrowIfCancellationRequested();
                    float modc = MaterialCensus.QuantizeRow(modcIndex);
                    if (!modelByBase.TryGetValue(formKey, out var model)) { noModel++; continue; }

                    IEnumerable<string> srcs = sourceKeysByGlb[GlbPath(model)];
                    if (lodByBase.TryGetValue(formKey, out var lodModel))
                        srcs = srcs.Concat(sourceKeysByGlb[GlbPath(lodModel)]);
                    string synthetic = $"MODC:{formKey}";
                    var covered = new HashSet<string>(StringComparer.Ordinal);
                    var rows = new List<object?[]>();

                    if (swapKey != null)
                    {
                        // Rare MODS+MODC combination (2 vanilla bases): the real
                        // swap's entries win per material, MODC colors the rest.
                        foreach (var e in await conn.QueryAsync(
                            "SELECT original_material, replacement_material, diffuse, normal, is_blend, alpha_mult, alpha_test, two_sided, material_name FROM material_swap_entries WHERE swap_form_key = @S",
                            new { S = swapKey }, transaction))
                        {
                            covered.Add((string)e.original_material);
                            rows.Add(new object?[] { synthetic, e.original_material, e.replacement_material,
                                e.diffuse, e.normal, e.is_blend, e.alpha_mult, e.alpha_test, e.two_sided, e.material_name });
                        }
                    }

                    bool anyModcEntry = false;
                    foreach (var src in srcs.Distinct(StringComparer.Ordinal))
                    {
                        if (covered.Contains(src)) continue;
                        if (!MaterialCensus.TryResolve(src, out var def) || !def.IsPalette) continue;

                        string texKey = MaterialCensus.NormalizeTexPath(def.Diffuse);
                        string baked = MaterialCensus.BakedDiffuseName(texKey, def.Lut, modc);
                        string diffuse;
                        if (baked != texKey)
                        {
                            diffuse = baked;
                            lateCombos.Add((texKey, def.Lut, modc));
                        }
                        else
                        {
                            // The MODC row IS the in-place winner: point back at the
                            // default texture (the GLB may reference a variant file
                            // when this material's own combo lost the winner vote).
                            diffuse = NifConverter.CleanTexturePathForDeploy(def.Diffuse, isDiffuse: true);
                        }

                        MaterialSwapProcessor.ParseAlphaToken(def.AlphaToken, out bool isBlend, out double alphaMult, out int? alphaTest);
                        string? normal = NifConverter.CleanTexturePathForDeploy(def.Normal, isDiffuse: false);
                        rows.Add(new object?[] { synthetic, src, def.Key, diffuse, normal,
                            isBlend ? 1 : 0, alphaMult, alphaTest, def.TwoSided ? 1 : 0,
                            MaterialNameCodec.ForDefinition(def, diffuse, normal) });
                        anyModcEntry = true;
                    }

                    // Nothing palette-flagged on the model (or only the real swap's
                    // rows, which the base already points at): nothing to do.
                    if (!anyModcEntry) { noPaletteMaterial++; continue; }

                    using (EsmTiming.Measure("modc.inserts"))
                    {
                        await conn.ExecuteAsync(
                            "INSERT OR REPLACE INTO material_swaps (form_key, editor_id) VALUES (@K, @E)",
                            new { K = synthetic, E = $"<MODC {modc:F3} synthetic>" }, transaction);
                        BatchInsert.Execute(conn, transaction, "material_swap_entries",
                            EsmColumns.MaterialSwapEntries,
                            rows.ToArray());
                        await conn.ExecuteAsync(
                            "UPDATE base_material_swaps SET material_swap = @S WHERE form_key = @K",
                            new { S = synthetic, K = formKey }, transaction);
                    }
                    synthesized++;
                    entriesWritten += rows.Count;
                }
            }

            var variantCombos = lateCombos.ToList();
            int baked2 = 0;
            if (texturesOut != null)
            {
                long bakesStart = EsmTiming.Now();
                baked2 = PaletteBaker.BakeCombos(materialsRoot, texturesOut, variantCombos, ct);
                EsmTiming.Add("modc.bakes", bakesStart, variantCombos.Count);
            }
            else if (variantCombos.Count > 0)
            {
                Console.WriteLine($"  WARNING: {variantCombos.Count} MODC palette combos need baking but no textures output was provided.");
            }

            var summary = new ModcSynthesisSummary(
                modcBases.Count, synthesized, entriesWritten, baked2, variantCombos.Count, noModel, noPaletteMaterial, unreadableGlbs,
                FailedBakes: texturesOut != null ? variantCombos.Count - baked2 : 0);
            Console.WriteLine($"  {summary}");
            return summary;
        }

        // In load order (rowid), so the synthetic rows come out in the order the ESM scan found the bases. A base whose swap is
        // already a synthetic key was synthesized by an earlier run; it is skipped so a second run adds nothing.
        private static async Task<List<ModcBase>> ReadModcBasesAsync(SqliteConnection conn, SqliteTransaction? transaction)
        {
            var rows = await conn.QueryAsync<(string FormKey, string? SwapFormKey, double ColorRemapIndex)>(
                @"SELECT form_key, material_swap, color_remap_index FROM base_material_swaps
                  WHERE color_remap_index IS NOT NULL AND (material_swap IS NULL OR material_swap NOT LIKE 'MODC:%')
                  ORDER BY rowid", transaction: transaction);
            return rows.Select(r => new ModcBase(r.FormKey, r.SwapFormKey, (float)r.ColorRemapIndex)).ToList();
        }

        // model_path lookup across every base table that has one.
        private static async Task<Dictionary<string, string>> ReadModelPathsAsync(SqliteConnection conn, SqliteTransaction? transaction)
        {
            var modelByBase = new Dictionary<string, string>(StringComparer.OrdinalIgnoreCase);
            foreach (var table in new[] { "statics", "movable_statics", "containers", "doors",
                                          "furniture", "activators", "flora", "trees",
                                          "static_collections", "items" })
            {
                try
                {
                    foreach (var row in await conn.QueryAsync<(string FormKey, string ModelPath)>(
                        $"SELECT form_key, model_path FROM {table} WHERE model_path IS NOT NULL", transaction: transaction))
                        modelByBase.TryAdd(row.FormKey, row.ModelPath);
                }
                catch (SqliteException ex)
                {
                    ReadFailureTally.Shared.Record("MODC model_path lookup (table without model_path)", ex, table);
                }
            }
            return modelByBase;
        }

        // Distinct LOD models (statics only) get the same MODC re-row so a
        // recolored object doesn't flip back to the default band at the
        // LOD ring. has_distant_lod-without-lod_model_path reuses the full
        // model GLB, which is already covered by the model lookup.
        private static async Task<Dictionary<string, string>> ReadLodModelPathsAsync(SqliteConnection conn, SqliteTransaction? transaction)
        {
            var lodByBase = new Dictionary<string, string>(StringComparer.OrdinalIgnoreCase);
            try
            {
                foreach (var row in await conn.QueryAsync<(string FormKey, string LodPath)>(
                    "SELECT form_key, lod_model_path FROM statics WHERE lod_model_path IS NOT NULL", transaction: transaction))
                    lodByBase.TryAdd(row.FormKey, row.LodPath);
            }
            catch (SqliteException ex)
            {
                ReadFailureTally.Shared.Record("MODC lod_model_path lookup (pre-LOD-backfill schema)", ex, "statics");
            }
            return lodByBase;
        }

        // Models are shared between bases (a crate model serves every color variant), so each GLB is opened once; the reads run
        // concurrently because they are latency-bound on freshly written files.
        // A model that cannot be read counts as having no palette materials; how many did is returned so the summary shows it.
        private static (Dictionary<string, List<string>> SourceKeys, int Unreadable) ReadSourceKeysOfAll(IEnumerable<string> glbPaths, CancellationToken ct)
        {
            var distinct = glbPaths.Distinct(StringComparer.OrdinalIgnoreCase).ToList();
            long start = EsmTiming.Now();
            var sourceKeys = OrderedParallel.Select(distinct, GlbReadParallelism, ReadGlbSourceKeys, ct);
            EsmTiming.Add("modc.glb-reads", start, distinct.Count);
            var byGlb = distinct.Zip(sourceKeys).ToDictionary(pair => pair.First, pair => pair.Second ?? [], StringComparer.OrdinalIgnoreCase);
            return (byGlb, sourceKeys.Count(keys => keys == null));
        }

        private static string GlbName(string model) =>
            model.EndsWith(".nif", StringComparison.OrdinalIgnoreCase) ? model[..^4] + ".glb" : model + ".glb";

        /// <summary>The "src:" tokens of the GLB's materials, or <c>null</c> when the file is missing or not a readable GLB.</summary>
        private static List<string>? ReadGlbSourceKeys(string glbPath)
        {
            var result = new List<string>();
            try
            {
                using var fs = File.OpenRead(glbPath);
                using var br = new BinaryReader(fs);
                if (br.ReadUInt32() != 0x46546C67) throw new InvalidDataException("not a glTF binary"); // "glTF"
                br.ReadUInt32(); // version
                br.ReadUInt32(); // total length
                uint chunkLen = br.ReadUInt32();
                br.ReadUInt32(); // chunk type ("JSON")
                var json = br.ReadBytes((int)chunkLen);
                using var doc = JsonDocument.Parse(json);
                if (doc.RootElement.TryGetProperty("materials", out var mats))
                {
                    foreach (var m in mats.EnumerateArray())
                    {
                        if (!m.TryGetProperty("name", out var nameEl)) continue;
                        var name = nameEl.GetString() ?? "";
                        int i = name.LastIndexOf("|src:", StringComparison.Ordinal);
                        if (i >= 0) result.Add(name[(i + 5)..]);
                    }
                }
            }
            catch (Exception ex) when (ex is IOException or UnauthorizedAccessException or JsonException or EndOfStreamException or InvalidDataException or ArgumentException)
            {
                Console.WriteLine($"  WARNING: unreadable GLB '{glbPath}' treated as having no palette materials: {ex.GetType().Name}: {ex.Message}");
                return null;
            }
            return result;
        }
    }
}
