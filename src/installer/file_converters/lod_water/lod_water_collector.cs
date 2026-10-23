using System.Globalization;
using System.Text.RegularExpressions;

namespace AssetConverter
{
    /// <summary>Two L4 water quads cover the same cell with different heights.</summary>
    public sealed class LodWaterConflictException : InvalidOperationException
    {
        public LodWaterConflictException(string message) : base(message) { }
    }

    public readonly record struct LodWaterCell(string World, int CellX, int CellY);

    /// <summary>What the collector saw, for the install log.</summary>
    public sealed record LodWaterReport(
        int FilesRead,
        int FilesWithWater,
        int QuadsRead,
        int DuplicatesWithinFile,
        int DuplicatesAcrossFiles,
        IReadOnlyDictionary<string, int> QuadsPerWorld);

    public sealed record LodWaterHeights(IReadOnlyDictionary<LodWaterCell, double> Heights, LodWaterReport Report);

    /// <summary>A terrain LOD file inside an archive: <c>Meshes\Terrain\&lt;World&gt;\&lt;World&gt;.&lt;level&gt;.&lt;x&gt;.&lt;y&gt;.BTR</c>.</summary>
    public readonly record struct BtrFileName(string World, int Level, int BlockX, int BlockY)
    {
        static readonly Regex Pattern = new(
            @"^meshes[\\/]terrain[\\/](?<world>[^\\/]+)[\\/]\k<world>\.(?<level>\d+)\.(?<x>-?\d+)\.(?<y>-?\d+)\.btr$",
            RegexOptions.IgnoreCase | RegexOptions.CultureInvariant);

        public static bool TryParse(string path, out BtrFileName name)
        {
            var match = Pattern.Match(path);
            if (!match.Success)
            {
                name = default;
                return false;
            }
            name = new BtrFileName(
                match.Groups["world"].Value,
                int.Parse(match.Groups["level"].Value, CultureInfo.InvariantCulture),
                int.Parse(match.Groups["x"].Value, CultureInfo.InvariantCulture),
                int.Parse(match.Groups["y"].Value, CultureInfo.InvariantCulture));
            return true;
        }
    }

    /// <summary>
    /// Merges the water quads of every level-4 terrain LOD file into one height per (worldspace, cell). A cell that
    /// appears more than once (several shapes in a file, or neighbouring files) is accepted when every occurrence
    /// has the same height and rejected with <see cref="LodWaterConflictException"/> otherwise.
    /// </summary>
    public static class LodWaterCollector
    {
        public const int WaterLodLevel = 4;

        public static LodWaterHeights Collect(IEnumerable<(string Path, Func<byte[]> ReadBytes)> entries)
        {
            var heights = new Dictionary<LodWaterCell, (double Height, string Source)>();
            int filesRead = 0, filesWithWater = 0, quadsRead = 0, withinFile = 0, acrossFiles = 0;

            foreach (var (path, readBytes) in entries)
            {
                if (!BtrFileName.TryParse(path, out var name) || name.Level != WaterLodLevel) continue;
                filesRead++;
                var quads = BtrWaterReader.Read(readBytes(), path, name.BlockX, name.BlockY);
                if (quads.Count > 0) filesWithWater++;
                quadsRead += quads.Count;

                foreach (var quad in quads)
                {
                    var cell = new LodWaterCell(name.World, quad.CellX, quad.CellY);
                    if (!heights.TryGetValue(cell, out var existing))
                    {
                        heights[cell] = (quad.Height, path);
                        continue;
                    }
                    if (existing.Height != quad.Height)
                        throw new LodWaterConflictException(string.Create(CultureInfo.InvariantCulture,
                            $"{name.World} cell ({quad.CellX},{quad.CellY}) has height {existing.Height} in {existing.Source} but {quad.Height} in {path}"));
                    if (existing.Source == path) withinFile++; else acrossFiles++;
                }
            }

            var perWorld = heights.Keys.GroupBy(c => c.World, StringComparer.OrdinalIgnoreCase)
                .OrderBy(g => g.Key, StringComparer.OrdinalIgnoreCase)
                .ToDictionary(g => g.Key, g => g.Count());
            return new LodWaterHeights(
                heights.ToDictionary(p => p.Key, p => p.Value.Height),
                new LodWaterReport(filesRead, filesWithWater, quadsRead, withinFile, acrossFiles, perWorld));
        }
    }
}
