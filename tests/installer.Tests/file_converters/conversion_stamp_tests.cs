using System.Text.Json;
using System.Text.Json.Nodes;
using AssetConverter;
using Xunit;

namespace InstallerTests
{
    public sealed class ConversionStampTests : IDisposable
    {
        private static readonly SourceFingerprint Movie = SourceFingerprint.Of(@"Interface\Menu.swf", [1, 2, 3, 4]);
        private static readonly SourceFingerprint Config = SourceFingerprint.Of(@"Interface\FontConfig.txt", [7]);
        private static readonly ConversionStamp Stamp = new(3, "en", [Movie, Config]);

        private readonly TempDirectory _root = new();

        public void Dispose() => _root.Dispose();

        private static string? Difference(ConversionStamp stamp, string recorded)
        {
            using JsonDocument document = JsonDocument.Parse(recorded);
            return stamp.Difference(document.RootElement);
        }

        [Fact]
        public void Fingerprint_IsTheEntrySizeAndLowercaseSha256()
        {
            Assert.Equal(new SourceFingerprint(@"Interface\Menu.swf", 4, "9f64a747e1b97f131fabb6b447296c9b6f0201e79fb3c5356e6c77e89b6a806a"), Movie);
        }

        [Fact]
        public void Difference_OfItsOwnRecord_IsNone()
        {
            Assert.Null(Difference(Stamp, Stamp.ToJson()));
        }

        [Fact]
        public void ToJson_OmitsAnAbsentLanguage()
        {
            Assert.False(JsonNode.Parse(new ConversionStamp(1, null, [Movie]).ToJson())!.AsObject().ContainsKey("language"));
        }

        private static readonly (ConversionStamp Current, string Reason)[] Changes =
        [
            (Stamp with { Extractor = 4 }, "it was converted by another extractor version"),
            (Stamp with { Language = "de" }, "it was converted for another language"),
            (Stamp with { Language = null }, "it was converted for another language"),
            (Stamp with { FfdecVersion = "1.0" }, "it was converted by another ffdec version"),
            (Stamp with { Sources = [Movie] }, "the game files it was converted from have changed"),
            (Stamp with { Sources = [Config, Movie] }, "the game files it was converted from have changed"),
            (Stamp with { Sources = [Movie, Config with { Sha256 = new string('0', 64) }] }, "the game files it was converted from have changed"),
        ];

        public static TheoryData<int> ChangeIndices => [.. Enumerable.Range(0, Changes.Length)];

        [Theory]
        [MemberData(nameof(ChangeIndices))]
        public void Difference_OfAnotherConversion_NamesWhatChanged(int change)
        {
            Assert.Equal(Changes[change].Reason, Difference(Changes[change].Current, Stamp.ToJson()));
        }

        [Theory]
        [InlineData("[]")]
        [InlineData("""{"extractor":"3","language":"en","sources":[]}""")]
        [InlineData("""{"extractor":3,"language":5,"sources":[]}""")]
        [InlineData("""{"extractor":3,"language":"en"}""")]
        public void Difference_OfAMalformedRecord_IsAReason(string recorded)
        {
            Assert.NotNull(Difference(Stamp, recorded));
        }

        [Fact]
        public void ToJson_RecordsTheFfdecVersionOnlyWhenTheOutputWentThroughIt()
        {
            Assert.False(JsonNode.Parse(Stamp.ToJson())!.AsObject().ContainsKey("ffdec"));
            Assert.Equal("26.2.1", JsonNode.Parse((Stamp with { FfdecVersion = "26.2.1" }).ToJson())!["ffdec"]!.GetValue<string>());
        }

        [Fact]
        public void Difference_OfARecordFromAnotherFfdecVersion_IsReported()
        {
            var withFfdec = Stamp with { FfdecVersion = "26.2.1" };

            Assert.Null(Difference(withFfdec, withFfdec.ToJson()));
            Assert.Equal("it was converted by another ffdec version", Difference(withFfdec with { FfdecVersion = "27.0.0" }, withFfdec.ToJson()));
            Assert.Equal("it was converted by another ffdec version", Difference(withFfdec, Stamp.ToJson()));
        }

        [Fact]
        public void Sidecar_OfAFile_RoundTrips()
        {
            string output = Path.Combine(_root.Path, "out.json");
            File.WriteAllText(output, "{}");

            Assert.Equal("it records no conversion", Stamp.SidecarStaleReason(output));
            Stamp.WriteSidecar(output);

            Assert.True(File.Exists(output + ConversionStamp.SidecarSuffix));
            Assert.Null(Stamp.SidecarStaleReason(output));
            Assert.Equal("it was converted for another language", (Stamp with { Language = "fr" }).SidecarStaleReason(output));
        }

        [Fact]
        public void Sidecar_OfAFolder_SitsInsideIt()
        {
            string output = Path.Combine(_root.Path, "fonts");
            Directory.CreateDirectory(output);

            Stamp.WriteSidecar(output);

            Assert.True(File.Exists(Path.Combine(output, ConversionStamp.SidecarSuffix)));
            Assert.Null(Stamp.SidecarStaleReason(output));
        }

        [Fact]
        public void SidecarStaleReason_MissingOutputOrUnreadableSidecar_IsAReason()
        {
            string output = Path.Combine(_root.Path, "out.json");
            Assert.Equal("it does not exist", Stamp.SidecarStaleReason(output));

            File.WriteAllText(output, "{}");
            File.WriteAllText(output + ConversionStamp.SidecarSuffix, "not json");

            Assert.StartsWith("its conversion record is not valid JSON", Stamp.SidecarStaleReason(output));
        }
    }
}
