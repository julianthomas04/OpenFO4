using System.Collections.Concurrent;
using Pfim;

namespace AssetConverter
{
    // FO4 greyscale-to-palette materials: the diffuse DDS stores luminance only
    // (decoded it looks purple/lavender — the classic "purple bricks") and the
    // BGSM points at a small gradient LUT via sGreyscaleTexture. The vanilla
    // shader replaces the sampled RGB with a LUT sample: X = luminance, Y = the
    // variant band selected by fGrayscaleToPaletteScale (overridden per swap /
    // record by MSWP ColorRemappingIndex / MODC). Axis semantics verified
    // against bricks01grad01 (luminance ramps run along X, variant bands stack
    // along Y) and the sportscar/stationwagon paint LUT.
    //
    // The engine has no palette shader, so bake the LUT into the deployed
    // diffuse instead. Each (diffuse, LUT, row) combo bakes to the filename
    // MaterialCensus.BakedDiffuseName agrees on with the NIF converter and the
    // ESM loader: the per-diffuse winner combo overwrites the deployed copy in
    // place (the pre-variant behavior), every other combo gets its own
    // ".pal-xxxxxxxx" variant file next to it.
    public static class PaletteBaker
    {
        /// <summary>
        /// Where to read source diffuse/LUT textures from. When the pipeline
        /// extracts texture archives directly into assets\textures (no staging
        /// copy), point this at texturesOut. Null = legacy staging\textures.
        /// </summary>
        public static string? SourceTexturesOverride { get; set; }

        /// <summary>
        /// When true, MSWP/MODC late-bake requests are queued instead of baked
        /// immediately — the pipelined installer runs the ESM load while
        /// textures are still extracting, so bakes must wait. Drain with
        /// <see cref="RunDeferredLateBakes"/> once textures are deployed.
        /// </summary>
        public static bool DeferLateBakes { get; set; }
        public static readonly ConcurrentQueue<(string Diffuse, string Lut, float Row)> PendingLateBakes = new();

        public static int RunDeferredLateBakes(string tempArchives, string texturesOut)
        {
            int ok = 0, total = 0;
            while (PendingLateBakes.TryDequeue(out var combo))
            {
                total++;
                if (BakeCombo(tempArchives, texturesOut, combo.Diffuse, combo.Lut, combo.Row)) ok++;
            }
            if (total > 0)
                Console.WriteLine($"Deferred palette bakes: {ok}/{total} done.");
            return total;
        }

        private static string ResolveSourceTexturesDir(string tempArchives)
            => SourceTexturesOverride ?? Path.Combine(tempArchives, "textures");

        // When bake sources live in texturesOut (staging-free layout), a
        // winner bake REPLACES the file later bakes need as their source.
        // Pristine originals are stashed here before being overwritten;
        // reads prefer the stash. Legacy staging layout never hits this
        // (sources and destinations are different trees).
        private static string? _pristineDir;

        private static string ResolveBakeSource(string sourceDir, string rel)
        {
            if (_pristineDir != null)
            {
                string stashed = Path.Combine(_pristineDir, rel);
                if (File.Exists(stashed)) return stashed;
            }
            return Path.Combine(sourceDir, rel);
        }

        private static void StashPristine(string sourceDir, string rel)
        {
            if (_pristineDir == null) return;
            string src = Path.Combine(sourceDir, rel);
            string dst = Path.Combine(_pristineDir, rel);
            if (File.Exists(dst) || !File.Exists(src)) return;
            Directory.CreateDirectory(Path.GetDirectoryName(dst)!);
            File.Copy(src, dst);
        }

        /// <summary>
        /// Runs concurrently with NIF conversion, the ESM load and Havok conversion in the full install, and every worker holds a
        /// decoded 2-4K texture plus its mip chain, so the fan-out is capped at half the cores. Groups are independent, so the
        /// cap changes no output.
        /// </summary>
        public static int BakeParallelism { get; } = Math.Max(1, Environment.ProcessorCount / 2);

        public static void BakeAll(string tempArchives, string texturesOut, CancellationToken ct = default)
        {
            string sourceDir = ResolveSourceTexturesDir(tempArchives);
            if (!Directory.Exists(sourceDir))
            {
                Console.WriteLine($"Palette bake: texture source not found at {sourceDir}, skipping.");
                return;
            }

            // Staging-free layout: sources and destinations are the same tree,
            // so originals about to be winner-overwritten get stashed for any
            // later bake (MSWP/MODC) that still needs the pristine pixels.
            if (SourceTexturesOverride != null)
            {
                _pristineDir = Path.Combine(tempArchives, "palette_sources");
                Directory.CreateDirectory(_pristineDir);
            }

            MaterialCensus.Build(tempArchives);
            var combos = MaterialCensus.PaletteCombos.ToList();
            if (combos.Count == 0)
            {
                Console.WriteLine("Palette bake: no greyscale-to-palette materials found, skipping.");
                return;
            }

            Console.WriteLine($"Palette bake: baking {combos.Count} (diffuse, LUT, row) combos...");
            int baked = 0, skipped = 0;
            object consoleLock = new();

            // Combos grouped per diffuse; within a group the in-place winner
            // bakes LAST so variant bakes never read a half-replaced source.
            var groups = combos.GroupBy(c => c.Diffuse).ToList();
            var options = new ParallelOptions { CancellationToken = ct, MaxDegreeOfParallelism = BakeParallelism };
            Parallel.ForEach(groups, options, group =>
            {
                var ordered = group.OrderBy(c =>
                    MaterialCensus.BakedDiffuseName(c.Diffuse, c.Lut, c.Row) == c.Diffuse ? 1 : 0).ToList();
                foreach (var (diffuseRel, lutRel, row) in ordered)
                {
                    ct.ThrowIfCancellationRequested();
                    string destRel = MaterialCensus.BakedDiffuseName(diffuseRel, lutRel, row);
                    string diffuseSrc = ResolveBakeSource(sourceDir, diffuseRel);
                    string lutSrc = ResolveBakeSource(sourceDir, lutRel);
                    string dest = Path.Combine(texturesOut, destRel);

                    try
                    {
                        if (!File.Exists(diffuseSrc) || !File.Exists(lutSrc))
                            throw new FileNotFoundException(File.Exists(diffuseSrc) ? lutSrc : diffuseSrc);

                        if (string.Equals(destRel, diffuseRel, StringComparison.OrdinalIgnoreCase))
                            StashPristine(sourceDir, diffuseRel);
                        BakeOne(diffuseSrc, lutSrc, dest, row);
                        Interlocked.Increment(ref baked);
                    }
                    catch (Exception ex)
                    {
                        // The plain extracted copy stays in place — worst case
                        // the texture keeps its purple palette-index look.
                        Interlocked.Increment(ref skipped);
                        lock (consoleLock)
                            Console.WriteLine($"  skipped {destRel} (LUT {lutRel}, row {row:F3}): {ex.Message}");
                    }
                }
            });

            Console.WriteLine($"Palette bake: {baked} baked, {skipped} skipped.");
        }

        /// <summary>
        /// Bake one combo discovered after the census pass — MSWP substitutions
        /// carrying a ColorRemappingIndex (and, later, MODC bases) select LUT
        /// rows no BGSM declares. Called from the ESM loader once swap entries
        /// are resolved. No-op when the target file already exists (the winner
        /// combo, or an extra requested twice).
        /// </summary>
        public static bool BakeCombo(string tempArchives, string texturesOut, string diffuse, string lut, float row)
        {
            string destRel = MaterialCensus.BakedDiffuseName(diffuse, lut, row);
            string dest = Path.Combine(texturesOut, destRel);
            if (File.Exists(dest)) return true;

            try
            {
                string sourceDir = ResolveSourceTexturesDir(tempArchives);
                BakeOne(ResolveBakeSource(sourceDir, diffuse), ResolveBakeSource(sourceDir, lut), dest, row);
                return true;
            }
            catch (Exception ex)
            {
                Console.WriteLine($"  palette bake (late) skipped {destRel}: {ex.Message}");
                return false;
            }
        }

        /// <summary>
        /// <see cref="BakeCombo"/> for many variant combos at once, at most <see cref="BakeParallelism"/> at a time; returns how many
        /// ended up baked. Combos that share a destination file bake one after another on a single worker, so no two workers ever
        /// write the same file. The combos must be variants (destination different from every combo's source diffuse), as late
        /// bakes are; an in-place winner bake overwrites a source other workers may be reading.
        /// </summary>
        public static int BakeCombos(string tempArchives, string texturesOut,
            IReadOnlyList<(string Diffuse, string Lut, float Row)> combos, CancellationToken ct = default)
        {
            var perDestination = combos
                .GroupBy(c => MaterialCensus.BakedDiffuseName(c.Diffuse, c.Lut, c.Row), StringComparer.OrdinalIgnoreCase)
                .Select(group => group.ToList())
                .ToList();
            return OrderedParallel.Select(perDestination, BakeParallelism,
                group => group.Count(c => BakeCombo(tempArchives, texturesOut, c.Diffuse, c.Lut, c.Row)), ct).Sum();
        }

        private static void BakeOne(string diffusePath, string lutPath, string destPath, float row)
        {
            using var diffuse = Pfimage.FromFile(diffusePath);
            using var lut = Pfimage.FromFile(lutPath);

            // One LUT row = one variant band: Y is selected by the row float,
            // X is swept by luminance (dark -> left, bright -> right).
            int lutY = Math.Clamp((int)(row * lut.Height), 0, lut.Height - 1);
            var lutRow = new (byte r, byte g, byte b)[lut.Width];
            for (int x = 0; x < lut.Width; x++)
            {
                var (r, g, b, _) = GetPixel(lut, x, lutY);
                lutRow[x] = (r, g, b);
            }

            int w = diffuse.Width, h = diffuse.Height;
            var baked = new byte[w * h * 4]; // tightly packed BGRA
            for (int y = 0; y < h; y++)
            {
                for (int x = 0; x < w; x++)
                {
                    var (r, g, b, a) = GetPixel(diffuse, x, y);
                    int gray = (299 * r + 587 * g + 114 * b) / 1000;
                    int col = Math.Min(gray * lut.Width / 256, lut.Width - 1);
                    var c = lutRow[col];
                    int o = (y * w + x) * 4;
                    baked[o] = c.b;
                    baked[o + 1] = c.g;
                    baked[o + 2] = c.r;
                    baked[o + 3] = a; // diffuse alpha survives (hair, decals)
                }
            }

            // Full mip chain via box filter so distant surfaces don't shimmer.
            var mips = new List<byte[]> { baked };
            int mw = w, mh = h;
            while (mw > 1 || mh > 1)
            {
                mips.Add(Downsample(mips[^1], mw, mh, out mw, out mh));
            }

            // Bake to a temp file and move over the destination: safe when the
            // destination IS the source diffuse (winner combos overwriting the
            // deployed copy in a staging-free install) and against hardlinked
            // destinations (truncating in place would reach through the link).
            // Temp name is unique per bake so concurrent combos never collide.
            Directory.CreateDirectory(Path.GetDirectoryName(destPath)!);
            string tmp = $"{destPath}.bake-tmp-{Environment.CurrentManagedThreadId}";
            try
            {
                WriteUncompressedBgraDds(tmp, w, h, mips);
                File.Move(tmp, destPath, overwrite: true);
            }
            catch
            {
                try { if (File.Exists(tmp)) File.Delete(tmp); } catch { }
                throw;
            }
        }

        private static (byte r, byte g, byte b, byte a) GetPixel(IImage img, int x, int y)
        {
            var data = img.Data;
            int o = y * img.Stride;
            switch (img.Format)
            {
                case ImageFormat.Rgba32: // Pfim's "Rgba32" is BGRA byte order
                    o += x * 4;
                    return (data[o + 2], data[o + 1], data[o], data[o + 3]);
                case ImageFormat.Rgb24:
                    o += x * 3;
                    return (data[o + 2], data[o + 1], data[o], 255);
                case ImageFormat.Rgb8:
                    o += x;
                    return (data[o], data[o], data[o], 255);
                default:
                    throw new NotSupportedException($"Unsupported decoded format {img.Format}");
            }
        }

        private static byte[] Downsample(byte[] src, int w, int h, out int nw, out int nh)
        {
            nw = Math.Max(1, w / 2);
            nh = Math.Max(1, h / 2);
            var dst = new byte[nw * nh * 4];
            for (int y = 0; y < nh; y++)
            {
                int sy0 = Math.Min(y * 2, h - 1), sy1 = Math.Min(y * 2 + 1, h - 1);
                for (int x = 0; x < nw; x++)
                {
                    int sx0 = Math.Min(x * 2, w - 1), sx1 = Math.Min(x * 2 + 1, w - 1);
                    int o = (y * nw + x) * 4;
                    for (int c = 0; c < 4; c++)
                    {
                        int sum = src[(sy0 * w + sx0) * 4 + c] + src[(sy0 * w + sx1) * 4 + c]
                                + src[(sy1 * w + sx0) * 4 + c] + src[(sy1 * w + sx1) * 4 + c];
                        dst[o + c] = (byte)(sum / 4);
                    }
                }
            }
            return dst;
        }

        // Legacy 128-byte DDS header, uncompressed 32-bit BGRA with mips. The
        // engine's DDS loader recognizes this by the pixel-format masks.
        private static void WriteUncompressedBgraDds(string path, int width, int height, List<byte[]> mips)
        {
            using var bw = new BinaryWriter(File.Create(path));
            bw.Write(0x20534444u);          // "DDS "
            bw.Write(124u);                 // dwSize
            bw.Write(0x2100Fu);             // CAPS|HEIGHT|WIDTH|PITCH|PIXELFORMAT|MIPMAPCOUNT
            bw.Write((uint)height);
            bw.Write((uint)width);
            bw.Write((uint)(width * 4));    // pitch of top mip
            bw.Write(0u);                   // depth
            bw.Write((uint)mips.Count);
            for (int i = 0; i < 11; i++) bw.Write(0u); // reserved
            bw.Write(32u);                  // ddspf dwSize
            bw.Write(0x41u);                // DDPF_RGB | DDPF_ALPHAPIXELS
            bw.Write(0u);                   // fourcc
            bw.Write(32u);                  // RGB bit count
            bw.Write(0x00FF0000u);          // R mask
            bw.Write(0x0000FF00u);          // G mask
            bw.Write(0x000000FFu);          // B mask
            bw.Write(0xFF000000u);          // A mask
            bw.Write(0x401008u);            // caps: COMPLEX|TEXTURE|MIPMAP
            bw.Write(0u); bw.Write(0u); bw.Write(0u); bw.Write(0u); // caps2-4, reserved2
            foreach (var mip in mips)
                bw.Write(mip);
        }
    }
}
