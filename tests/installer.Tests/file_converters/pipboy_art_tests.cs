using System.IO.Compression;
using System.Text;
using System.Text.Json;
using AssetConverter;
using Xunit;

namespace InstallerTests
{
    // docs/installer/interface.md: the SWF reader, the SVG rebasing and the manifest the engine validates
    // (src/engine/src/assets/pipboy_art.rs).
    public class PipboyArtTests
    {
        const string ExportedFrame =
            "<?xml version=\"1.0\" encoding=\"UTF-8\" standalone=\"no\"?>\n" +
            "<svg xmlns:ffdec=\"https://www.free-decompiler.com/flash\" xmlns:xlink=\"http://www.w3.org/1999/xlink\" ffdec:objectType=\"frame\" height=\"11.8px\" width=\"7.15px\" xmlns=\"http://www.w3.org/2000/svg\">\n" +
            "  <g transform=\"matrix(1.0, 0.0, 0.0, 1.0, 3.55, 5.9)\">\n    <use ffdec:characterId=\"36\" xlink:href=\"#shape0\"/>\n  </g>\n  <defs><g id=\"shape0\"><path d=\"M0 0\"/></g></defs>\n</svg>";

        const string EmptyFrame =
            "<?xml version=\"1.0\"?>\n<svg xmlns=\"http://www.w3.org/2000/svg\" height=\"0.0px\" width=\"0.0px\"></svg>";

        [Theory]
        [InlineData(0.0)]
        public void FrameRate_NotPositive_Throws(double rate)
        {
            var movie = new SwfMovieBuilder { FrameRate = rate }.Read();

            var failure = Assert.Throws<PipboyArtException>(() => PipboyArtExporter.FrameRate(movie, "Boy.swf"));

            Assert.Contains("Boy.swf", failure.Message);
        }

        [Fact]
        public void FrameRate_IsTheHeaderRate()
        {
            Assert.Equal(24.0, PipboyArtExporter.FrameRate(new SwfMovieBuilder { FrameRate = 24 }.Read(), "Boy.swf"));
        }

        [Fact]
        public void SvgFrame_Bounds_UndoTheRegistrationPointOffset()
        {
            var bounds = SvgFrame.Parse(ExportedFrame).Bounds;

            Assert.Equal(new ArtBounds(-3.55, -5.9, 3.6, 5.9), bounds);
        }

        [Fact]
        public void SvgFrame_Rebase_SetsViewBoxToBoundsAndRemovesTheOffset()
        {
            var frame = SvgFrame.Parse(ExportedFrame);

            string svg = frame.Rebase(new ArtBounds(-5, -6, 5, 6));

            Assert.Contains("viewBox=\"-5 -6 10 12\"", svg);
            Assert.Contains("<g transform=\"matrix(1.0, 0.0, 0.0, 1.0, 0.0, 0.0)\">", svg);
            Assert.DoesNotContain("3.55", svg);
            Assert.DoesNotContain("px", svg);
            System.Xml.Linq.XDocument.Parse(svg);
        }

        [Fact]
        public void SvgFrame_WithoutCanvas_IsEmptyButStillRebases()
        {
            var frame = SvgFrame.Parse(EmptyFrame);

            Assert.Null(frame.Bounds);
            Assert.Contains("viewBox=\"0 0 4 4\"", frame.Rebase(new ArtBounds(0, 0, 4, 4)));
        }

        const string BlankFrame =
            "<?xml version=\"1.0\"?>\n<svg xmlns=\"http://www.w3.org/2000/svg\" height=\"400.0px\" width=\"550.0px\">\n" +
            "  <g transform=\"matrix(1.0, 0.0, 0.0, 1.0, 0.0, 0.0)\"/>\n</svg>";

        [Theory]
        [InlineData("<svg xmlns=\"http://www.w3.org/2000/svg\" width=\"10px\"><g transform=\"matrix(1.0, 0.0, 0.0, 1.0, 0.0, 0.0)\"><path d=\"M0 0\"/></g></svg>")]
        [InlineData("<svg xmlns=\"http://www.w3.org/2000/svg\" width=\"10px\" height=\"wide\"><g transform=\"matrix(1.0, 0.0, 0.0, 1.0, 0.0, 0.0)\"><path d=\"M0 0\"/></g></svg>")]
        [InlineData("<svg xmlns=\"http://www.w3.org/2000/svg\" width=\"NaN\" height=\"5px\"></svg>")]
        public void SvgFrame_MissingOrUnreadableSize_Throws(string svg)
        {
            Assert.Throws<SvgFormatException>(() => SvgFrame.Parse(svg));
        }

        [Fact]
        public void SvgFrame_SizeWithoutAUnit_IsInPixels()
        {
            var frame = SvgFrame.Parse(ExportedFrame.Replace("11.8px", "11.8").Replace("7.15px", "7.15"));

            Assert.Equal(new ArtBounds(-3.55, -5.9, 3.6, 5.9), frame.Bounds);
        }

        [Fact]
        public void SvgFrame_EmptyGroupInsideTheContent_DoesNotMakeTheFrameBlank()
        {
            const string nested = "<svg xmlns=\"http://www.w3.org/2000/svg\" height=\"20.0px\" width=\"10.0px\"><g transform=\"matrix(1.0, 0.0, 0.0, 1.0, 5.0, 10.0)\">"
                + "<g transform=\"matrix(1.0, 0.0, 0.0, 1.0, 1.0, 1.0)\"/><path d=\"M0 0\"/></g></svg>";

            var frame = SvgFrame.Parse(nested);

            Assert.Equal(new ArtBounds(-5, -10, 5, 10), frame.Bounds);
            Assert.Contains("<path", frame.Rebase(new ArtBounds(-5, -10, 5, 10)));
        }

        [Fact]
        public void SvgFrame_BlankFrameWithEmptyOriginGroup_IsEmptyButStillRebases()
        {
            var frame = SvgFrame.Parse(BlankFrame);

            Assert.Null(frame.Bounds);
            string svg = frame.Rebase(new ArtBounds(0, 0, 550, 400));
            Assert.Contains("viewBox=\"0 0 550 400\"", svg);
            System.Xml.Linq.XDocument.Parse(svg);
        }

        [Fact]
        public void SvgFrame_WithCanvasButNoOriginGroup_Throws()
        {
            string svg = "<svg height=\"4.0px\" width=\"4.0px\"><path d=\"M0 0\"/></svg>";

            Assert.Throws<SvgFormatException>(() => SvgFrame.Parse(svg));
        }

        [Fact]
        public void ParseFile_BrokenFrame_NamesClipAndFile()
        {
            string directory = Directory.CreateTempSubdirectory().FullName;
            try
            {
                string file = Path.Combine(directory, "65.svg");
                File.WriteAllText(file, "<svg height=\"4.0px\" width=\"4.0px\"/>");

                var failure = Assert.Throws<PipboyArtException>(() => PipboyArtExporter.ParseFile(file, "clip 'vaultboy/x'"));

                Assert.Contains("vaultboy/x", failure.Message);
                Assert.Contains("65.svg", failure.Message);
            }
            finally
            {
                Directory.Delete(directory, true);
            }
        }

        static PipboyArtClip Clip(string id, params string[] frames) =>
            new(id, "Source.swf", null, ArtColour.Mask, 30, [], new ArtBounds(0, 0, 10, 20), frames);

        static void Publish(string artRoot, IEnumerable<PipboyArtClip> clips, IReadOnlyCollection<string> requiredClipIds)
        {
            var writer = new PipboyArtWriter(artRoot);
            foreach (var clip in clips) writer.Add(clip);
            writer.Complete(requiredClipIds);
        }

        static string SvgFor(string marker) =>
            SvgFrame.Parse(ExportedFrame.Replace("#shape0", marker)).Rebase(new ArtBounds(0, 0, 10, 20));

        static string ReadFrame(string artRoot, string relativePath)
        {
            string path = Path.Combine(artRoot, relativePath);
            if (!relativePath.EndsWith(".svgz", StringComparison.Ordinal)) return File.ReadAllText(path);
            using var gzip = new GZipStream(File.OpenRead(path), CompressionMode.Decompress);
            using var reader = new StreamReader(gzip);
            return reader.ReadToEnd();
        }

        /// <summary>The art extractor version and the SHA-256 of the manifest and frame files the writer makes for the clips below.</summary>
        static readonly (int ExtractorVersion, string Sha256) Golden = (4, "ec8e5fda3814ebb231c97d1561a00a8e6c49bfaf7b2559e5a4339b0b2f867273");

        [Fact]
        public void Writer_ChangedOutput_RaisesTheExtractorVersion()
        {
            using var dir = new TempDirectory();
            PipboyArtClip[] clips =
            [
                Clip("a/one", SvgFor("#a"), SvgFor("#b")) with { Script = [new(1, "stop", null)], Offset = [1.5, 2], Tint = [0.5, 0.5, 0.5, 1] },
                Clip("a/two", SvgFor("#a")),
            ];

            Publish(dir.Path, clips, []);

            var lines = Directory.GetFiles(dir.Path, "*", SearchOption.AllDirectories)
                .Select(file => Path.GetRelativePath(dir.Path, file).Replace(Path.DirectorySeparatorChar, '/') + ":" + Convert.ToHexString(System.Security.Cryptography.SHA256.HashData(File.ReadAllBytes(file))))
                .Order(StringComparer.Ordinal);
            string written = Convert.ToHexString(System.Security.Cryptography.SHA256.HashData(Encoding.UTF8.GetBytes(string.Join("|", lines)))).ToLowerInvariant();

            Assert.True(Golden == (PipboyArtExtractor.ExtractorVersion, written),
                $"The art output changed (SHA-256 {written}). Installed art is reused unless PipboyArtExtractor.ExtractorVersion is raised: "
                + "raise it and record the new version and hash in Golden.");
        }

        [Fact]
        public void Writer_Manifest_CarriesTheContractFields()
        {
            using var dir = new TempDirectory();
            var clip = new PipboyArtClip("perkcard/0", "LevelUpMenu.swf", @"Components\VaultBoys\Perks\PerkClip_00c0de.swf",
                ArtColour.Full, 30, [new(1, "stop", null), new(1, "gotoAndPlay", 1)], new ArtBounds(-115, -80, 115, 80), [SvgFor("#a")]);

            Publish(dir.Path, [clip], []);

            using var manifest = JsonDocument.Parse(File.ReadAllText(Path.Combine(dir.Path, InstallLayout.PipboyArtManifestFileName)));
            Assert.Equal(3, manifest.RootElement.GetProperty("format").GetInt32());
            var written = manifest.RootElement.GetProperty("clips")[0];
            Assert.Equal("perkcard/0", written.GetProperty("id").GetString());
            Assert.Equal("LevelUpMenu.swf", written.GetProperty("source").GetString());
            Assert.Equal(@"Components\VaultBoys\Perks\PerkClip_00c0de.swf", written.GetProperty("illustrates").GetString());
            Assert.Equal("full", written.GetProperty("colour").GetString());
            Assert.Equal(30.0, written.GetProperty("fps").GetDouble());
            Assert.Equal("""[{"frame":1,"action":"stop"},{"frame":1,"action":"gotoAndPlay","target":1}]""", written.GetProperty("script").GetRawText());
            Assert.Equal([-115.0, -80.0, 115.0, 80.0], written.GetProperty("bounds").EnumerateArray().Select(v => v.GetDouble()));
        }

        [Fact]
        public void Writer_PlacedPart_CarriesOffsetAndTintAndOtherClipsOmitThem()
        {
            using var dir = new TempDirectory();
            var part = Clip("doll/body", SvgFor("#a")) with { Offset = [1.25, 0.15], Tint = [0.3, 0.3, 0.3, 1] };

            Publish(dir.Path, [part, Clip("icon/weight", SvgFor("#b"))], []);

            using var manifest = JsonDocument.Parse(File.ReadAllText(Path.Combine(dir.Path, InstallLayout.PipboyArtManifestFileName)));
            var clips = manifest.RootElement.GetProperty("clips");
            Assert.Equal([1.25, 0.15], clips[0].GetProperty("offset").EnumerateArray().Select(v => v.GetDouble()));
            Assert.Equal([0.3, 0.3, 0.3, 1.0], clips[0].GetProperty("tint").EnumerateArray().Select(v => v.GetDouble()));
            Assert.False(clips[1].TryGetProperty("offset", out _));
            Assert.False(clips[1].TryGetProperty("tint", out _));
        }

        [Fact]
        public void Writer_ClipWithoutIllustratedMovie_OmitsTheField()
        {
            using var dir = new TempDirectory();

            Publish(dir.Path, [Clip("icon/weight", SvgFor("#a"))], []);

            Assert.DoesNotContain("illustrates", File.ReadAllText(Path.Combine(dir.Path, InstallLayout.PipboyArtManifestFileName)));
        }

        [Fact]
        public void Writer_RepeatedFrames_ShareOneFile()
        {
            using var dir = new TempDirectory();
            string a = SvgFor("#a"), b = SvgFor("#b");

            Publish(dir.Path, [Clip("special/strength", a, a, b, a)], []);

            using var manifest = JsonDocument.Parse(File.ReadAllText(Path.Combine(dir.Path, InstallLayout.PipboyArtManifestFileName)));
            var frames = manifest.RootElement.GetProperty("clips")[0].GetProperty("frames").EnumerateArray().Select(f => f.GetString()!).ToList();
            Assert.Equal(frames[0], frames[1]);
            Assert.Equal(frames[0], frames[3]);
            Assert.NotEqual(frames[0], frames[2]);
            Assert.Equal(2, Directory.GetFiles(Path.Combine(dir.Path, "special", "strength")).Length);
            Assert.StartsWith("special/strength/0000.", frames[0]);
            Assert.StartsWith("special/strength/0002.", frames[2]);
        }

        [Fact]
        public void Writer_FramesCarryTheClipBoundsAsViewBox()
        {
            using var dir = new TempDirectory();
            var clip = new PipboyArtClip("icon/weight", "PipboyMenu.swf", null, ArtColour.Mask, 30, [], new ArtBounds(-5.5, -5.5, 5.5, 5.5),
                [SvgFrame.Parse(ExportedFrame).Rebase(new ArtBounds(-5.5, -5.5, 5.5, 5.5))]);

            Publish(dir.Path, [clip], []);

            using var manifest = JsonDocument.Parse(File.ReadAllText(Path.Combine(dir.Path, InstallLayout.PipboyArtManifestFileName)));
            string frame = manifest.RootElement.GetProperty("clips")[0].GetProperty("frames")[0].GetString()!;
            Assert.Contains("viewBox=\"-5.5 -5.5 11 11\"", ReadFrame(dir.Path, frame));
        }

        [Fact]
        public void Writer_LargeRepetitiveFrame_IsGzipped()
        {
            using var dir = new TempDirectory();
            string big = SvgFor("#a").Replace("<path d=\"M0 0\"/>", string.Concat(Enumerable.Repeat("<path d=\"M0 0 L10 10 L20 0 Z\"/>", 200)));

            Publish(dir.Path, [Clip("vaultboy/x", big)], []);

            Assert.Single(Directory.GetFiles(Path.Combine(dir.Path, "vaultboy", "x"), "*.svgz"));
        }

        [Fact]
        public void Writer_FolderWithTrailingBlankInId_IsSanitised()
        {
            Assert.Equal("vaultboy/components/perks/sample", PipboyArtWriter.ClipFolder("vaultboy/components/perks/sample "));
            Assert.Equal("a/b_c", PipboyArtWriter.ClipFolder("a/b:c"));
        }

        [Fact]
        public void Writer_MissingRequiredClip_Throws()
        {
            using var dir = new TempDirectory();

            var error = Assert.Throws<PipboyArtException>(() =>
                Publish(dir.Path, [Clip("icon/weight", SvgFor("#a"))], ["icon/weight", "doll/body"]));

            Assert.Contains("doll/body", error.Message);
        }

        [Fact]
        public void Manifest_FormatProblem_ReportsAFrameFileThatIsMissingOrEmpty()
        {
            using var dir = new TempDirectory();
            Publish(dir.Path, [Clip("a/b", SvgFor("#a"))], ["a/b"]);
            string manifest = Path.Combine(dir.Path, InstallLayout.PipboyArtManifestFileName);
            string frame = Directory.GetFiles(Path.Combine(dir.Path, "a", "b")).Single();

            Assert.Null(PipboyArtWriter.FormatProblem(manifest));

            File.WriteAllBytes(frame, []);
            Assert.Contains("missing or empty", PipboyArtWriter.FormatProblem(manifest));
            File.Delete(frame);
            Assert.Contains("missing or empty", PipboyArtWriter.FormatProblem(manifest));
        }

        [Fact]
        public void Writer_DuplicateId_Throws()
        {
            using var dir = new TempDirectory();

            Assert.Throws<PipboyArtException>(() =>
                Publish(dir.Path, [Clip("a/b", SvgFor("#a")), Clip("a/b", SvgFor("#b"))], []));
        }

        [Fact]
        public void Writer_ClipsAddedOutOfOrder_AreListedByIdAndWrittenWhenAdded()
        {
            using var dir = new TempDirectory();
            var writer = new PipboyArtWriter(dir.Path);

            writer.Add(Clip("b/two", SvgFor("#b")));
            Assert.Single(Directory.GetFiles(Path.Combine(dir.Path, "b", "two")));
            writer.Add(Clip("a/one", SvgFor("#a")));
            Assert.Equal(2, writer.Complete(["a/one"]));

            using var manifest = JsonDocument.Parse(File.ReadAllText(Path.Combine(dir.Path, InstallLayout.PipboyArtManifestFileName)));
            Assert.Equal(["a/one", "b/two"], manifest.RootElement.GetProperty("clips").EnumerateArray().Select(c => c.GetProperty("id").GetString()));
        }

        [Fact]
        public void Writer_ClipsAddedConcurrently_AllReachTheManifest()
        {
            using var dir = new TempDirectory();
            var writer = new PipboyArtWriter(dir.Path);

            Parallel.For(0, 32, index => writer.Add(Clip($"c/{index:D2}", SvgFor("#a"))));

            Assert.Equal(32, writer.Complete([]));
        }

        [Fact]
        public void Writer_FolderCollidingIds_ThrowBeforeAnythingIsOverwritten()
        {
            using var dir = new TempDirectory();
            var writer = new PipboyArtWriter(dir.Path);
            writer.Add(Clip("a/b", SvgFor("#a")));
            string frame = Directory.GetFiles(Path.Combine(dir.Path, "a", "b")).Single();
            byte[] before = File.ReadAllBytes(frame);

            Assert.Throws<PipboyArtException>(() => writer.Add(Clip("a/B", SvgFor("#b"))));

            Assert.Equal(before, File.ReadAllBytes(frame));
        }

        [Fact]
        public void Writer_InvalidClips_Throw()
        {
            using var dir = new TempDirectory();
            var noFrames = Clip("a/none");
            var noRate = Clip("a/rate", SvgFor("#a")) with { FramesPerSecond = 0 };
            var flat = Clip("a/flat", SvgFor("#a")) with { Bounds = new ArtBounds(0, 0, 0, 5) };

            var stray = Clip("a/stray", SvgFor("#a")) with { Script = [new(1, "gotoAndPlay", 2)] };

            foreach (var bad in new[] { noFrames, noRate, flat, stray })
                Assert.Throws<PipboyArtException>(() => Publish(dir.Path, [bad], []));
        }

        [Fact]
        public void Catalog_IconAndDollIds_AreUnique()
        {
            var ids = PipboyArtCatalog.Icons.Select(s => s.ClipId)
                .Concat(PipboyArtCatalog.DollPieces.Select(s => s.ClipId))
                .Concat(PipboyArtCatalog.NamedSymbols.Select(s => s.ClipId))
                .Concat(PipboyArtCatalog.PerkChartSymbols.Select(s => s.ClipId))
                .Concat(PipboyArtCatalog.InterfaceSymbols.Select(s => s.ClipId))
                .Concat(PipboyArtCatalog.DamageIcons(6).Select(s => s.ClipId)).ToList();

            Assert.Equal(ids.Count, ids.Distinct().Count());
        }

        [Fact]
        public void Catalog_MarkerSources_FollowTheClassListOrder()
        {
            var markers = PipboyArtCatalog.MarkerSources(["Alpha", "Beta"]);

            Assert.Equal(["marker/0", "marker/1"], markers.Select(m => m.ClipId));
            Assert.Equal(["Alpha", "Beta"], markers.Select(m => m.ClassName));
        }

        [Fact]
        public void Catalog_VaultBoyClipId_NormalisesPathAndTrimsBlanks()
        {
            Assert.Equal("vaultboy/components/vaultboys/perks/sample",
                PipboyArtCatalog.VaultBoyClipId(@"Components\VaultBoys\Perks\Sample .swf"));
        }

        [Fact]
        public void Catalog_PerkMoviePath_IsDerivedFromTheCardClass()
        {
            Assert.Equal(@"Components\VaultBoys\Perks\PerkClip_00c0de.swf", PipboyArtCatalog.PerkMoviePath("PerkClip00c0de"));
        }

        [Fact]
        public void InstallLayout_PipboyManifest_SitsBelowTheInterfacePipboyFolder()
        {
            string manifest = InstallLayout.PipboyArtManifestFile("assets");

            Assert.Equal(Path.Combine("assets", "interface", "pipboy", "art_manifest.json"), manifest);
        }
    }
}
