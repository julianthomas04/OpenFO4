using System.Collections.Concurrent;
using System.Diagnostics.CodeAnalysis;
using System.Globalization;

namespace AssetConverter
{
    /// <summary>One parsed material file from the extracted Materials tree.</summary>
    /// <param name="Key">Census key: "materials/setdressing/vault/vault_crate_01.bgsm" (lowercase, forward slashes).</param>
    /// <param name="Diffuse">Raw diffuse path as authored (BGSM DiffuseTexture / BGEM BaseTexture).</param>
    /// <param name="Normal">Raw normal path as authored.</param>
    /// <param name="AlphaToken">"" | "blend" | "blend|0.50" | "test:{0-255}" — same grammar as GLB material names.</param>
    /// <param name="TwoSided">Back-face culling disabled.</param>
    /// <param name="Decal">Coplanar overlay (BGSM/BGEM bDecal, bDepthBias, or bDecalNoFade):
    /// the runtime routes it into a depth-biased pipeline so it doesn't z-fight its host
    /// surface. bDecalNoFade is a decal sub-flag, so it implies decal treatment too.</param>
    /// <param name="IsEffect">.bgem (effect shader) rather than .bgsm.</param>
    /// <param name="IsPalette">Grayscale-to-palette BGSM whose LUT exists in the staging Textures tree.</param>
    /// <param name="Lut">LUT path relative to the Textures root ("" unless IsPalette).</param>
    /// <param name="Row">Quantized fGrayscaleToPaletteScale — the LUT V coordinate selecting the variant band.</param>
    /// <param name="Lighting">Specular and emissive inputs (raw texture paths); None for effect materials.</param>
    /// <param name="Effect">Effect-shader parameters of a .bgem (null for .bgsm).</param>
    /// <param name="Env">Environment-map inputs; null when mapping is off or its cube texture is not in the texture index.</param>
    public sealed record MaterialDef(
        string Key,
        string Diffuse,
        string Normal,
        string AlphaToken,
        bool TwoSided,
        bool Decal,
        bool IsEffect,
        bool IsPalette,
        string Lut,
        float Row,
        LightingParams Lighting,
        EffectMaterialParams? Effect = null,
        EnvironmentParams? Env = null);

    /// <summary>
    /// Census of every BGSM/BGEM in the extracted archives. Single source of
    /// truth shared by three pipeline stages that must agree byte-for-byte on
    /// material identity and palette-baked texture names:
    ///   - NifConverter (GLB material names, incl. the "src:" census key),
    ///   - PaletteBaker (which (diffuse, LUT, row) combos to bake and where),
    ///   - ESMLoader's MaterialSwapProcessor (MSWP substitution resolution).
    /// Built once per process from the staging root; read-only afterwards.
    ///
    /// Palette semantics (verified against the shipped LUTs): a LUT's X axis is the luminance ramp
    /// and its Y axis stacks variant bands. The band is selected by the BGSM's
    /// fGrayscaleToPaletteScale, overridden per swap/record by MSWP
    /// ColorRemappingIndex / MODC. Variants of one diffuse therefore differ
    /// only in the V coordinate ("row") used at bake time.
    /// </summary>
    public static class MaterialCensus
    {
        private static readonly object _buildLock = new();
        private static string? _builtRoot;
        private static Dictionary<string, MaterialDef> _byKey = new();
        private static Dictionary<string, List<MaterialDef>> _byFileName = new();

        // Palette bookkeeping, keyed by Textures-root-relative diffuse path.
        private static Dictionary<string, Dictionary<(string Lut, float Row), int>> _combosByDiffuse = new();
        private static Dictionary<string, int> _nonPaletteUsers = new();
        private static Dictionary<string, (string Lut, float Row)> _winnerByDiffuse = new();
        private static HashSet<string> _rawDominated = new();

        public static bool IsBuilt => _builtRoot != null;
        public static int Count => _byKey.Count;

        /// <summary>
        /// How the census decides a LUT texture "exists". Default: a file
        /// under sourceRoot\textures (the legacy staging layout). The
        /// pipelined installer extracts textures straight into assets\textures
        /// — and builds the census BEFORE texture bytes hit disk — so it
        /// points this at the texture index seeded from BA2 name tables.
        /// Takes a NormalizeTexPath-form relative path.
        /// </summary>
        public static Func<string, bool>? TextureExistsOverride { get; set; }

        /// <summary>All distinct (diffuse, LUT, row) combos declared by BGSMs.
        /// MSWP ColorRemappingIndex / MODC combos are not census-declared and
        /// are baked late via PaletteBaker.BakeCombo.</summary>
        /// <summary>Materials that asked for an environment map whose cube texture is not in the texture index.</summary>
        public static int MissingEnvCubeCount => _missingEnvCubes;
        private static int _missingEnvCubes;

        private static bool TextureExists(string normalizedPath) =>
            TextureExistsOverride?.Invoke(normalizedPath)
            ?? File.Exists(Path.Combine(_stagingTextures, normalizedPath));
        private static string _stagingTextures = "";

        /// <summary>Existence check for environment-map textures, shared by the census and the NIF converter's
        /// inline path: the repaired path is looked up in the texture index when one is available, otherwise
        /// the staged texture tree decides.</summary>
        public static bool EnvTextureExists(string normalizedPath) =>
            NifConverter.TextureIndexAvailable
                ? NifConverter.TextureResolves(normalizedPath)
                : TextureExists(normalizedPath);

        public static IEnumerable<(string Diffuse, string Lut, float Row)> PaletteCombos =>
            _combosByDiffuse.SelectMany(d => d.Value.Keys.Select(c => (d.Key, c.Lut, c.Row)));

        /// <summary>Idempotent. Safe to call from any stage (converter, baker, ESM load).</summary>
        public static void Build(string sourceRoot)
        {
            lock (_buildLock)
            {
                if (_builtRoot != null)
                {
                    if (!string.Equals(_builtRoot, sourceRoot, StringComparison.OrdinalIgnoreCase))
                        Console.WriteLine($"Material census: WARNING — already built from {_builtRoot}, ignoring rebuild from {sourceRoot}.");
                    return;
                }
                BuildInner(sourceRoot);
                _builtRoot = sourceRoot;
            }
        }

        /// <summary>Forgets the built census so the next <see cref="Build"/> starts over; lets tests leave the process-wide state as they found it.</summary>
        internal static void Reset()
        {
            lock (_buildLock)
            {
                _builtRoot = null;
                _byKey = new();
                _byFileName = new();
                _combosByDiffuse = new();
                _nonPaletteUsers = new();
                _winnerByDiffuse = new();
                _rawDominated = new();
                _missingEnvCubes = 0;
                _stagingTextures = "";
            }
        }

        private static void BuildInner(string sourceRoot)
        {
            string stagingTextures = Path.Combine(sourceRoot, "textures");
            _stagingTextures = stagingTextures;
            _missingEnvCubes = 0;
            string[] bgsmFiles, bgemFiles;
            try
            {
                bgsmFiles = Directory.GetFiles(sourceRoot, "*.bgsm", SearchOption.AllDirectories);
                bgemFiles = Directory.GetFiles(sourceRoot, "*.bgem", SearchOption.AllDirectories);
            }
            catch (Exception ex)
            {
                Console.WriteLine($"Material census: failed to scan {sourceRoot}: {ex.Message}");
                return;
            }

            if (bgsmFiles.Length == 0 && bgemFiles.Length == 0)
            {
                Console.WriteLine("Material census: 0 files found (Materials BA2 not extracted?).");
                return;
            }

            Console.WriteLine($"Material census: parsing {bgsmFiles.Length} .bgsm + {bgemFiles.Length} .bgem files...");
            var defs = new ConcurrentBag<MaterialDef>();
            int bgsmFailed = 0, bgemFailed = 0;

            Parallel.ForEach(bgsmFiles, file =>
            {
                try
                {
                    using var stream = new FileStream(file, FileMode.Open, FileAccess.Read);
                    var bgsm = new BGSM();
                    bgsm.Open(stream);

                    string key = NormalizeMaterialKey(Path.GetRelativePath(sourceRoot, file));
                    string lut = NormalizeTexPath(bgsm.GreyscaleTexture);
                    bool lutExists = TextureExists(lut);
                    bool isPalette = bgsm.GrayscaleToPaletteColor && lut.Length > 0 && lutExists;
                    defs.Add(new MaterialDef(
                        Key: key,
                        Diffuse: bgsm.DiffuseTexture ?? "",
                        Normal: bgsm.NormalTexture ?? "",
                        AlphaToken: GetAlphaToken(bgsm),
                        TwoSided: bgsm.TwoSided,
                        Decal: bgsm.Decal || bgsm.DepthBias || bgsm.DecalNoFade,
                        IsEffect: false,
                        IsPalette: isPalette,
                        Lut: isPalette ? lut : "",
                        Row: isPalette ? QuantizeRow(bgsm.GrayscaleToPaletteScale) : 0f,
                        Lighting: LightingParams.FromBgsm(bgsm),
                        Env: ResolveEnvironment(bgsm.EnvironmentMapping, bgsm.EnvmapTexture, null, bgsm.EnvironmentMappingMaskScale)));
                }
                catch (Exception e) when (BaseMaterialFile.IsReadFailure(e))
                {
                    Interlocked.Increment(ref bgsmFailed);
                    Console.WriteLine($"[WARN] Material census: skipped '{file}': {e.Message}");
                }
            });

            Parallel.ForEach(bgemFiles, file =>
            {
                try
                {
                    using var stream = new FileStream(file, FileMode.Open, FileAccess.Read);
                    var bgem = new BGEM();
                    bgem.Open(stream);

                    string key = NormalizeMaterialKey(Path.GetRelativePath(sourceRoot, file));
                    defs.Add(new MaterialDef(
                        Key: key,
                        Diffuse: bgem.BaseTexture ?? "",
                        Normal: bgem.NormalTexture ?? "",
                        AlphaToken: GetAlphaToken(bgem),
                        TwoSided: bgem.TwoSided,
                        Decal: bgem.Decal || bgem.DepthBias || bgem.DecalNoFade,
                        IsEffect: true,
                        IsPalette: false,
                        Lut: "",
                        Row: 0f,
                        Lighting: LightingParams.None,
                        Effect: EffectMaterialParams.FromBgem(bgem),
                        Env: ResolveEnvironment(bgem.EnvironmentMapping, bgem.EnvmapTexture, bgem.EnvmapMaskTexture, bgem.EnvironmentMappingMaskScale)));
                }
                catch (Exception e) when (BaseMaterialFile.IsReadFailure(e))
                {
                    Interlocked.Increment(ref bgemFailed);
                    Console.WriteLine($"[WARN] Material census: skipped '{file}': {e.Message}");
                }
            });

            foreach (var def in defs)
            {
                _byKey[def.Key] = def;
                string name = Path.GetFileName(def.Key);
                if (!_byFileName.TryGetValue(name, out var list))
                    _byFileName[name] = list = new List<MaterialDef>();
                list.Add(def);
            }

            // Palette vote: which (LUT, row) combos exist per diffuse, and does
            // the raw (non-palette) look dominate? Same dominance rule as the
            // old PaletteBaker voting — a diffuse mostly used as a plain RGB
            // texture must keep its original file untouched.
            foreach (var def in defs)
            {
                string diffuse = NormalizeTexPath(def.Diffuse);
                if (diffuse.Length == 0 || def.IsEffect) continue;
                if (def.IsPalette)
                {
                    if (!_combosByDiffuse.TryGetValue(diffuse, out var combos))
                        _combosByDiffuse[diffuse] = combos = new Dictionary<(string, float), int>();
                    combos[(def.Lut, def.Row)] = combos.GetValueOrDefault((def.Lut, def.Row)) + 1;
                }
                else
                {
                    _nonPaletteUsers[diffuse] = _nonPaletteUsers.GetValueOrDefault(diffuse) + 1;
                }
            }

            foreach (var (diffuse, combos) in _combosByDiffuse)
            {
                int paletteCount = combos.Values.Sum();
                if (_nonPaletteUsers.GetValueOrDefault(diffuse) > paletteCount)
                {
                    _rawDominated.Add(diffuse);
                    continue;
                }
                // Most-used combo bakes in place (keeps today's filenames for
                // the single-combo majority); ties break deterministically.
                _winnerByDiffuse[diffuse] = combos
                    .OrderByDescending(c => c.Value)
                    .ThenBy(c => c.Key.Lut, StringComparer.Ordinal)
                    .ThenBy(c => c.Key.Row)
                    .First().Key;
            }

            int variantCombos = _combosByDiffuse.Sum(d => d.Value.Count) - _winnerByDiffuse.Count;
            Console.WriteLine($"Material census: {_byKey.Count} materials ({bgsmFailed + bgemFailed} failed), " +
                              $"{_combosByDiffuse.Count} palette diffuses, {variantCombos} variant combos, " +
                              $"{_rawDominated.Count} raw-dominated, {_missingEnvCubes} env cubes missing from the texture index.");
        }

        private static EnvironmentParams? ResolveEnvironment(bool enabled, string? cube, string? mask, float maskScale)
        {
            var env = EnvironmentParams.Resolve(enabled, cube, mask, maskScale, EnvTextureExists, out bool cubeMissing);
            if (cubeMissing) Interlocked.Increment(ref _missingEnvCubes);
            return env;
        }

        /// <summary>Exact resolution with the materials/-prefix tolerance the
        /// NIF converter has always applied to shader.Name references.</summary>
        public static bool TryResolve(string reference, [NotNullWhen(true)] out MaterialDef? def)
        {
            def = null;
            if (_byKey.Count == 0 || string.IsNullOrWhiteSpace(reference)) return false;

            string key = NormalizeMaterialKey(reference);
            if (_byKey.TryGetValue(key, out def)) return true;
            if (!key.StartsWith("materials/", StringComparison.Ordinal))
                return _byKey.TryGetValue("materials/" + key, out def);
            return _byKey.TryGetValue(key.Substring("materials/".Length), out def);
        }

        /// <summary>
        /// Lenient resolution for MSWP substitution paths, which are authored
        /// sloppily: bare filenames ("Sportscar01_Rust.BGSM"), missing
        /// materials/ prefixes, mixed case. Tries exact/prefixed first, then
        /// the directory of <paramref name="siblingKey"/> (the substitution's
        /// other side), then unique-filename lookup. May return multiple defs
        /// when a bare filename is ambiguous — for swap ORIGINALS, emitting
        /// one entry per candidate is correct (non-matching ones are inert).
        /// </summary>
        public static List<MaterialDef> ResolveLoose(string reference, string? siblingKey = null)
        {
            var results = new List<MaterialDef>();
            if (string.IsNullOrWhiteSpace(reference)) return results;

            if (TryResolve(reference, out var def))
            {
                results.Add(def);
                return results;
            }

            string key = NormalizeMaterialKey(reference);
            string fileName = Path.GetFileName(key);

            if (siblingKey != null)
            {
                string? dir = Path.GetDirectoryName(siblingKey)?.Replace('\\', '/');
                if (!string.IsNullOrEmpty(dir) && _byKey.TryGetValue($"{dir}/{fileName}", out def))
                {
                    results.Add(def);
                    return results;
                }
            }

            if (_byFileName.TryGetValue(fileName, out var byName))
                results.AddRange(byName);
            return results;
        }

        /// <summary>
        /// Deployed filename for a palette diffuse baked with a given (LUT, row)
        /// combo. The per-diffuse winner combo (most BGSM users) keeps the
        /// original filename — it is baked in place exactly like the old
        /// single-winner behavior. Every other combo, and every combo of a
        /// raw-dominated diffuse, gets a deterministic ".pal-xxxxxxxx" variant
        /// file. All inputs are Textures-root-relative (NormalizeTexPath form).
        /// </summary>
        public static string BakedDiffuseName(string diffuse, string lut, float row)
        {
            row = QuantizeRow(row);
            if (!_rawDominated.Contains(diffuse) &&
                _winnerByDiffuse.TryGetValue(diffuse, out var winner) &&
                winner.Lut == lut && winner.Row == row)
                return diffuse;
            return VariantName(diffuse, lut, row);
        }

        public static string VariantName(string diffuse, string lut, float row)
        {
            string stem = diffuse.EndsWith(".dds", StringComparison.OrdinalIgnoreCase)
                ? diffuse[..^4]
                : diffuse;
            return $"{stem}.pal-{Hash8($"{lut}|{QuantizeRow(row).ToString("F3", CultureInfo.InvariantCulture)}")}.dds";
        }

        /// <summary>Rows quantized to 3 decimals so converter, baker, and ESM
        /// loader always derive identical variant filenames from a float.</summary>
        public static float QuantizeRow(float row) =>
            float.IsFinite(row) ? MathF.Round(row, 3) : 0f;

        private static string Hash8(string s)
        {
            // FNV-1a 32-bit: stable across runs and processes (string.GetHashCode is not).
            uint h = 2166136261;
            foreach (char c in s)
            {
                h ^= c;
                h *= 16777619;
            }
            return h.ToString("x8", CultureInfo.InvariantCulture);
        }

        public static string NormalizeMaterialKey(string path)
        {
            var lower = path.Replace('\\', '/').ToLowerInvariant().TrimStart('/');
            // NIF shader.Name often contains an absolute Bethesda build path like
            // "c:/projects/fallout4/build/pc/data/materials/architecture/foo.bgsm".
            // Strip everything before the last "materials/" segment so it aligns
            // with the census keys (which are relative to temp/archives/).
            int idx = lower.LastIndexOf("materials/", StringComparison.Ordinal);
            return idx >= 0 ? lower.Substring(idx) : lower;
        }

        /// <summary>Texture path relative to the Textures root: lowercase,
        /// forward slashes, no leading "textures/". Matches the frame the
        /// palette baker reads from and writes to.</summary>
        public static string NormalizeTexPath(string? path)
        {
            if (string.IsNullOrEmpty(path)) return "";
            var clean = path.Replace('\\', '/').ToLowerInvariant().TrimStart('/');
            return clean.StartsWith("textures/", StringComparison.Ordinal) ? clean["textures/".Length..] : clean;
        }

        // Blend mode takes priority over alpha-test when the material explicitly requests
        // blending (glass tiles, effect shaders, etc.). AlphaTest alone (foliage, rebar,
        // chain-link) uses scissor, which does depth writes and avoids sorting.
        public static string GetAlphaToken(BaseMaterialFile mat)
        {
            bool isEffect = mat is BGEM;
            bool wantsBlend = mat.AlphaBlendMode == BaseMaterialFile.AlphaBlendModeType.Standard ||
                              mat.AlphaBlendMode == BaseMaterialFile.AlphaBlendModeType.Additive ||
                              mat.AlphaBlendMode == BaseMaterialFile.AlphaBlendModeType.Multiplicative ||
                              mat.Alpha < 0.999f ||
                              (mat is BGEM bgem && bgem.GlassEnabled);
            bool hasAlphaScale = isEffect
                ? MathF.Abs(mat.Alpha - 1f) > AlphaScaleTolerance
                : mat.Alpha < 0.999f;

            if (wantsBlend)
                // InvariantCulture: the engine parses this float back out of the
                // material name; a comma-decimal OS locale must not leak into it.
                return !hasAlphaScale ? "blend"
                    : isEffect ? $"blend|{mat.Alpha.ToString("0.####", CultureInfo.InvariantCulture)}"
                    : $"blend|{mat.Alpha.ToString("F2", CultureInfo.InvariantCulture)}";

            if (mat.AlphaTest)
                return $"test:{mat.AlphaTestRef}";

            return "";
        }

        public const float AlphaScaleTolerance = 0.001f;
    }
}
