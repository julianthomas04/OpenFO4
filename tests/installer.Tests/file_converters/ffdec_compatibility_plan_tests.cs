// Copyright 2026 Julian Thomas and OpenFO4 contributors
// SPDX-License-Identifier: Apache-2.0
using System.Globalization;
using System.Security.Cryptography;
using System.Text.Json;
using AssetConverter;
using AssetConverter.Swf;
using AssetConverter.UiLayout;
using Xunit;

namespace InstallerTests;

public sealed class FfdecCompatibilityPlanTests
{
    private static readonly JsonSerializerOptions PlanJsonOptions = new()
    {
        WriteIndented = true,
        PropertyNamingPolicy = JsonNamingPolicy.CamelCase
    };

    private sealed record Job(string Id, string Category, string Input, IReadOnlyList<string> Covers, IReadOnlyList<string> Options, int? ExpectedFiles);
    private sealed record SymbolInventory(string Id, string Input, int CharacterId, IReadOnlyList<int> Frames);
    private sealed record MovieInventory(string Id, string Input, int FrameCount, int Sublength);

    [Fact(Explicit = true)]
    public void WriteEveryRequiredExportCommandFromTheCurrentInstallPlan()
    {
        string? game = Environment.GetEnvironmentVariable("OPENFO4_FO4_PATH");
        string? destination = Environment.GetEnvironmentVariable("OPENFO4_FFDEC_COMPATIBILITY_DIR");
        Assert.SkipWhen(string.IsNullOrWhiteSpace(game) || string.IsNullOrWhiteSpace(destination), "Set the game and ignored compatibility output paths explicitly.");
        string root = InstallerPaths.FindRepoRoot([AppContext.BaseDirectory]);
        string output = Path.GetFullPath(destination!);
        Assert.StartsWith(Path.GetFullPath(Path.Combine(root, "build")) + Path.DirectorySeparatorChar, output, StringComparison.OrdinalIgnoreCase);
        Assert.False(Directory.Exists(output), "Use a fresh compatibility directory.");
        Directory.CreateDirectory(output);
        var inputHashes = new SortedDictionary<string, string>(StringComparer.Ordinal);
        string Input(string name, byte[] content)
        {
            string relative = "inputs/" + name.Replace('\\', '/');
            string path = Path.GetFullPath(Path.Combine(output, relative));
            Assert.StartsWith(output + Path.DirectorySeparatorChar, path, StringComparison.OrdinalIgnoreCase);
            Directory.CreateDirectory(Path.GetDirectoryName(path)!);
            File.WriteAllBytes(path, content);
            inputHashes[relative] = Convert.ToHexString(SHA256.HashData(content)).ToLowerInvariant();
            return relative;
        }

        string settings = Environment.GetEnvironmentVariable("OPENFO4_FO4_SETTINGS")
            ?? Path.Combine(Environment.GetFolderPath(Environment.SpecialFolder.MyDocuments), "My Games", "Fallout4");
        string language = GameLanguage.Resolve(game!, settings);
        var configs = InterfaceArchive.ReadMatching(game!, path => string.Equals(path, UiLayoutExtractor.LanguageFontConfigEntry(language), StringComparison.OrdinalIgnoreCase)
            || string.Equals(path, UiLayoutExtractor.DefaultFontConfigEntry, StringComparison.OrdinalIgnoreCase));
        (string configPath, byte[] config) = UiLayoutExtractor.SelectFontConfig(configs, language);
        var libraries = FontConfig.Parse(GameText.Decode(config)).FontLibraries;
        var fonts = InterfaceArchive.ReadMatching(game!, path => libraries.Contains(path, StringComparer.OrdinalIgnoreCase));
        Assert.Equal(libraries.Count, fonts.Count);
        Assert.NotEmpty(fonts);
        var jobs = new List<Job>();
        var fontInventory = new List<string>();
        foreach (var (path, content) in fonts.OrderBy(font => font.Path, StringComparer.Ordinal))
        {
            string input = Input(path, content);
            fontInventory.Add(input);
            jobs.Add(new Job($"font-{jobs.Count}", "font", input, [input], ["-export", "font"], null));
        }

        var movies = PipboyArtExtractor.ReadMovies(game!);
        var plan = PipboyArtPlan.Build(movies);
        var symbolInventory = new List<SymbolInventory>();
        foreach (ResolvedClip clip in PipboyArtPlan.ResolveAll(movies, plan))
        {
            ArtMovie movie = PipboyArtPlan.Require(movies, clip.Work.Movie);
            symbolInventory.Add(new SymbolInventory(clip.Work.ClipId, Input("Interface/" + movie.Source, movie.Content),
                clip.Symbol.CharacterId, clip.Symbol.Frames.Distinct().Order().ToList()));
        }
        foreach (var group in plan.Symbols.GroupBy(work => work.Movie, StringComparer.OrdinalIgnoreCase))
        {
            ArtMovie movie = PipboyArtPlan.Require(movies, group.Key);
            var tree = new PipboySwfTree(SwfMovieReader.Read(movie.Content));
            var requests = PipboyArtPlan.Resolve(movie, tree, group).GroupBy(clip => clip.Symbol.CharacterId)
                .Select(symbol => new SymbolRequest(symbol.Key, symbol.SelectMany(clip => clip.Symbol.Frames).Distinct().Order().ToList())).ToList();
            string ids = string.Join(',', requests.Select(request => request.CharacterId.ToString(CultureInfo.InvariantCulture)));
            string frames = string.Join(',', requests.Select(request => $"{request.CharacterId}:{PipboyArtExporter.FrameRanges(request.Frames)}"));
            jobs.Add(new Job($"symbols-{jobs.Count}", "symbol", Input("Interface/" + movie.Source, movie.Content), group.Select(work => work.ClipId).ToList(),
                ["-format", "sprite:svg,shape:svg", "-ignorebackground", "-selectid", ids, "-select", frames, "-export", "sprite,shape"],
                requests.Sum(request => request.Frames.Count)));
        }
        var movieInventory = new List<MovieInventory>();
        foreach (MovieClipSource clip in plan.Movies)
        {
            ArtMovie movie = PipboyArtPlan.Require(movies, clip.Movie);
            SwfMovie parsed = SwfMovieReader.Read(movie.Content);
            int longest = parsed.Characters.Values.OfType<SwfSprite>().Select(sprite => sprite.Timeline.DeclaredFrameCount).DefaultIfEmpty(0).Max();
            bool nested = parsed.Root.DeclaredFrameCount == 1 && longest > 1;
            int count = nested ? longest : parsed.Root.DeclaredFrameCount;
            var options = new List<string> { "-format", "frame:svg", "-ignorebackground" };
            if (nested) options.AddRange(["-sublength", count.ToString(CultureInfo.InvariantCulture)]);
            options.AddRange(["-export", "frame"]);
            string input = Input("Interface/" + movie.Source, movie.Content);
            movieInventory.Add(new MovieInventory(clip.ClipId, input, count, nested ? count : 1));
            jobs.Add(new Job($"movie-{jobs.Count}", "movie", input, [clip.ClipId], options, count));
        }
        Assert.NotEmpty(jobs);
        var document = new
        {
            schema = 1,
            evidenceProtocol = 2,
            scope = "required-install-inputs",
            language,
            fontConfig = new { path = Input(configPath, config), sha256 = Convert.ToHexString(SHA256.HashData(config)).ToLowerInvariant() },
            producerHashes = Directory.GetFiles(Path.Combine(root, "src", "installer", "file_converters"), "*.cs", SearchOption.AllDirectories)
                .Append(Path.Combine(root, "tests", "installer.Tests", "file_converters", "ffdec_compatibility_plan_tests.cs"))
                .Order(StringComparer.Ordinal).ToDictionary(path => Path.GetRelativePath(root, path).Replace('\\', '/'),
                    path => Convert.ToHexString(SHA256.HashData(File.ReadAllBytes(path))).ToLowerInvariant(), StringComparer.Ordinal),
            expectedInventory = new { fonts = fontInventory, symbols = symbolInventory, movies = movieInventory },
            inputHashes,
            jobs
        };
        File.WriteAllText(Path.Combine(output, "plan.json"), JsonSerializer.Serialize(document, PlanJsonOptions));
    }
}
