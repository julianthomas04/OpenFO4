using System.Text.Json;
using System.Text.Json.Nodes;
using AssetConverter;
using AssetConverter.UiLayout;
using Xunit;

namespace InstallerTests.UiLayout
{
    public sealed class UiDocumentHeaderTests : IDisposable
    {
        private static readonly ConversionStamp Stamp = UiDocumentHeader.Stamp(null, SourceFingerprint.Of(@"Interface\Menu.swf", [1, 2, 3, 4]));

        private readonly TempDirectory _root = new();

        public void Dispose() => _root.Dispose();

        private string PathOf => Path.Combine(_root.Path, "Menu.json");

        private static string Document(string kind, ConversionStamp stamp)
        {
            using var buffer = new MemoryStream();
            using (var json = new Utf8JsonWriter(buffer))
            {
                json.WriteStartObject();
                UiDocumentHeader.Write(json, kind, stamp);
                json.WriteEndObject();
            }
            return System.Text.Encoding.UTF8.GetString(buffer.ToArray());
        }

        private string? Stale(string content)
        {
            File.WriteAllText(PathOf, content);
            return UiDocumentHeader.StaleReason(PathOf, UiDocumentHeader.MovieKind, Stamp);
        }

        [Fact]
        public void Write_StatesTheFormatKindAndConversion()
        {
            JsonNode document = JsonNode.Parse(Document(UiDocumentHeader.MovieKind, Stamp))!;

            Assert.Equal(UiDocumentHeader.Format, (int)document["format"]!);
            Assert.Equal("movie", (string)document["kind"]!);
            Assert.Equal(UiDocumentHeader.ExtractorVersion, (int)document["conversion"]!["extractor"]!);
            Assert.Equal(@"Interface\Menu.swf", (string)document["conversion"]!["sources"]![0]!["entry"]!);
        }

        [Fact]
        public void StaleReason_CurrentDocument_IsNone()
        {
            Assert.Null(Stale(Document(UiDocumentHeader.MovieKind, Stamp)));
        }

        [Fact]
        public void StaleReason_Missing_SaysSo()
        {
            Assert.Equal("it does not exist", UiDocumentHeader.StaleReason(PathOf, UiDocumentHeader.MovieKind, Stamp));
        }

        [Theory]
        [InlineData("not json")]
        [InlineData("[]")]
        [InlineData("""{"format":"1"}""")]
        [InlineData("""{"format":1,"kind":"movie"}""")]
        public void StaleReason_NotADocument_NamesAReason(string content)
        {
            Assert.NotNull(Stale(content));
        }

        [Fact]
        public void StaleReason_OtherFormat_IsStale()
        {
            JsonNode document = JsonNode.Parse(Document(UiDocumentHeader.MovieKind, Stamp))!;
            document["format"] = 999;

            Assert.Equal("it was written for another document format", Stale(document.ToJsonString()));
        }

        [Fact]
        public void StaleReason_OtherKind_IsStale()
        {
            Assert.Equal("it is not a movie document", Stale(Document(UiDocumentHeader.TranslateKind, Stamp)));
        }

        [Fact]
        public void StaleReason_OtherConversion_IsStale()
        {
            Assert.Equal("the game files it was converted from have changed",
                Stale(Document(UiDocumentHeader.MovieKind, UiDocumentHeader.Stamp(null, SourceFingerprint.Of(@"Interface\Menu.swf", [5])))));
        }
    }
}
