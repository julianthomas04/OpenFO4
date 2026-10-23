using System.Text;
using AssetConverter;
using Xunit;

namespace InstallerTests
{
    public sealed class CollisionNifListTests : IDisposable
    {
        private readonly TempDirectory _root = new();

        public void Dispose() => _root.Dispose();

        private string Source => Path.Combine(_root.Path, "Meshes");

        private string Nif(params string[] segments) => Path.Combine([Source, .. segments]);

        [Fact]
        public void TheFileHoldsOneRelativePathPerLineAsUtf8WithoutByteOrderMarkEndedByLineFeeds()
        {
            using var list = CollisionNifList.Create(Source, [Nif("a", "one.nif"), Nif("two.nif")]);

            byte[] bytes = File.ReadAllBytes(list.FilePath);

            Assert.Equal(Encoding.UTF8.GetBytes("a\\one.nif\ntwo.nif\n"), bytes);
        }

        [Fact]
        public void NonAsciiAndSpacedPathsSurviveAsUtf8()
        {
            string[] relatives = [@"Ünïcode\Gebäude Süd.nif", @"日本語\モデル.nif", @"with space\a b.nif", @"emoji\café ☃.nif"];
            using var list = CollisionNifList.Create(Source, relatives.Select(relative => Path.Combine(Source, relative)));

            var lines = File.ReadAllText(list.FilePath, new UTF8Encoding(false, throwOnInvalidBytes: true)).Split('\n');

            Assert.Equal([.. relatives, ""], lines);
            Assert.NotEqual(0xEF, File.ReadAllBytes(list.FilePath)[0]);
        }

        [Fact]
        public void AnEmptyListWritesAnEmptyFile()
        {
            using var list = CollisionNifList.Create(Source, []);

            Assert.Empty(File.ReadAllBytes(list.FilePath));
        }

        [Fact]
        public void APathOutsideTheSourceRootIsRejected()
        {
            Assert.Throws<ArgumentException>(() => CollisionNifList.Create(Source, [Nif("ok.nif"), Path.Combine(_root.Path, "Other", "x.nif")]));
            Assert.Throws<ArgumentException>(() => CollisionNifList.Create(Source, [Path.Combine(_root.Path, "Meshes2", "x.nif")]));
        }

        [Fact]
        public void DisposingDeletesTheFile()
        {
            var list = CollisionNifList.Create(Source, [Nif("one.nif")]);
            Assert.True(File.Exists(list.FilePath));

            list.Dispose();

            Assert.False(File.Exists(list.FilePath));
        }
    }
}
