using System.Text;
using AssetConverter;
using Xunit;

namespace InstallerTests
{
    public sealed class StreamingByteSearchTests
    {
        private static readonly byte[] Marker = Encoding.ASCII.GetBytes("bhkPhysicsSystem");

        private static byte[] Padding(int length) => Enumerable.Repeat((byte)'x', length).ToArray();

        private static byte[] Around(int before, int after) => [.. Padding(before), .. Marker, .. Padding(after)];

        private static bool Search(byte[] content, params int[] chunkSizes)
        {
            var search = new StreamingByteSearch(Marker);
            int offset = 0, next = 0;
            while (offset < content.Length)
            {
                int size = Math.Min(chunkSizes[next++ % chunkSizes.Length], content.Length - offset);
                search.Feed(content.AsSpan(offset, size));
                offset += size;
            }
            return search.Found;
        }

        [Fact]
        public void TheMarkerIsFoundWhereverItSitsInASingleChunk()
        {
            foreach (int before in new[] { 0, 1, 15, 16, 17, 1000 })
                Assert.True(Search(Around(before, 3), int.MaxValue), $"marker after {before} bytes");
        }

        [Fact]
        public void TheMarkerIsFoundAtEveryBoundaryOfTwoChunks()
        {
            var content = Around(40, 40);
            for (int split = 1; split < content.Length; split++)
                Assert.True(Search(content, split, content.Length), $"split at {split}");
        }

        [Theory]
        [InlineData(1)]
        [InlineData(2)]
        [InlineData(3)]
        [InlineData(7)]
        [InlineData(15)]
        [InlineData(16)]
        [InlineData(17)]
        public void TheMarkerIsFoundForEveryFixedChunkSizeAndAlignment(int chunk)
        {
            for (int before = 0; before < 40; before++)
                Assert.True(Search(Around(before, 5), chunk), $"chunk {chunk}, marker after {before} bytes");
        }

        [Fact]
        public void ABoundaryThatFallsInsideTheMarkerAcrossSeveralTinyChunksStillMatches()
        {
            Assert.True(Search(Around(5, 0), 1, 2, 5, 3));
        }

        [Theory]
        [InlineData(1)]
        [InlineData(5)]
        [InlineData(16)]
        [InlineData(4096)]
        public void ContentWithoutTheMarkerIsNeverFound(int chunk)
        {
            Assert.False(Search(Padding(5000), chunk));
            Assert.False(Search([.. Marker[..15], .. Padding(10), .. Marker[1..]], chunk));
            Assert.False(Search([.. Marker.Reverse(), .. Marker.Reverse()], chunk));
        }

        [Fact]
        public void APartialMarkerFollowedByTheRestOfADifferentWordIsNotAMatch()
        {
            Assert.False(Search(Encoding.ASCII.GetBytes("bhkPhysicsSyste" + "m".ToUpperInvariant()), 4));
            Assert.False(Search(Encoding.ASCII.GetBytes("bhkphysicssystem"), 16));
        }

        [Fact]
        public void EmptyChunksAreIgnoredAndAFoundMarkerStaysFound()
        {
            var search = new StreamingByteSearch(Marker);
            search.Feed([]);
            Assert.False(search.Found);
            search.Feed(Marker);
            search.Feed([]);
            search.Feed(Padding(100));
            Assert.True(search.Found);
        }

        [Fact]
        public void AMarkerSpreadOverMoreChunksThanItHasBytesIsStillFound()
        {
            var search = new StreamingByteSearch(Marker);
            foreach (byte b in Marker)
                search.Feed([b]);
            Assert.True(search.Found);
        }

        [Fact]
        public void ASingleBytePatternMatchesAnywhere()
        {
            var search = new StreamingByteSearch("q"u8);
            search.Feed("abc"u8);
            Assert.False(search.Found);
            search.Feed("xqz"u8);
            Assert.True(search.Found);
        }

        [Fact]
        public void AnEmptyPatternIsRejected()
        {
            Assert.Throws<ArgumentException>(() => new StreamingByteSearch([]));
        }
    }
}
