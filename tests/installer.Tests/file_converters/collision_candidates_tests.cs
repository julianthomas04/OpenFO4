using System.Text;
using System.Text.RegularExpressions;
using AssetConverter;
using InstallerTests.Support;
using Xunit;

namespace InstallerTests
{
    public sealed class CollisionCandidatesTests
    {
        private static readonly byte[] Marker = Encoding.ASCII.GetBytes(CollisionCandidates.PhysicsSystemMarker);

        private static byte[] NifWith(params string[] blockTypes) =>
            Encoding.ASCII.GetBytes("Gamebryo File Format\n" + string.Join('\0', blockTypes) + "\0tail");

        private static bool Verdict(byte[] content, params int[] chunkSizes)
        {
            const string nifPath = @"C:\staging\Meshes\a.nif";
            var candidates = new CollisionCandidates();
            var observer = candidates.Observe(nifPath);
            int offset = 0, next = 0;
            while (offset < content.Length)
            {
                int size = Math.Min(chunkSizes[next++ % chunkSizes.Length], content.Length - offset);
                observer.Write(content.AsSpan(offset, size));
                offset += size;
            }
            observer.Complete();
            return candidates.Select([nifPath]).Length == 1;
        }

        [Fact]
        public void TheMarkerIsTheBlockTypeNameTheWorkerLooksFor()
        {
            string worker = RepoFiles.ExtractBlock("src/installer/python/fo4_collision_worker.py",
                @"_PHYSICS_SYSTEM_BLOCK_TYPE = b""([^""]*)""", "the worker's physics system block type constant");

            Assert.Equal(CollisionCandidates.PhysicsSystemMarker, worker);
        }

        [Fact]
        public void TheWorkerPrefiltersByTheSameWholeFileByteSearch()
        {
            string rule = RepoFiles.ExtractBlock("src/installer/python/fo4_collision_worker.py",
                @"def _may_contain_collision\(nif_bytes: bytes\) -> bool:.*?(return [^\r\n]+)", "the worker's prefilter");

            Assert.Equal("return _PHYSICS_SYSTEM_BLOCK_TYPE in nif_bytes", rule);
        }

        [Fact]
        public void ANifWithTheMarkerIsACandidateForEveryChunking()
        {
            var nif = NifWith("NiNode", "bhkNPCollisionObject", "bhkPhysicsSystem", "BSTriShape");
            foreach (int chunk in new[] { 1, 3, 15, 16, 17, 64, int.MaxValue })
                Assert.True(Verdict(nif, chunk), $"chunk {chunk}");
        }

        [Fact]
        public void ANifWithoutTheMarkerIsNotACandidateForEveryChunking()
        {
            var nif = NifWith("NiNode", "bhkNPCollisionObject", "bhkPhysicsSyste", "BSTriShape", "hkPhysicsSystem");
            foreach (int chunk in new[] { 1, 3, 15, 16, 17, 64, int.MaxValue })
                Assert.False(Verdict(nif, chunk), $"chunk {chunk}");
        }

        [Fact]
        public void TheVerdictEqualsTheWorkersWholeFileRuleOnEveryMarkerPositionAndChunking()
        {
            var body = Enumerable.Range(0, 300).Select(i => (byte)(i * 7)).ToArray();
            for (int position = 0; position <= body.Length; position += 11)
            {
                byte[] with = [.. body[..position], .. Marker, .. body[position..]];
                foreach (int chunk in new[] { 1, 5, 16, 100, 1000 })
                {
                    Assert.Equal(Contains(with, Marker), Verdict(with, chunk));
                    Assert.Equal(Contains(body, Marker), Verdict(body, chunk));
                }
            }
        }

        private static bool Contains(byte[] content, byte[] marker) => content.AsSpan().IndexOf(marker) >= 0;

        [Fact]
        public void AnEmptyFileIsNotACandidate()
        {
            Assert.False(Verdict([], 1));
        }

        [Fact]
        public void AStagedNifNobodyObservedIsACandidateSoTheWorkerDecidesIt()
        {
            var candidates = new CollisionCandidates();
            candidates.Observe(@"C:\s\Meshes\seen_no.nif").Write(NifWith("NiNode"));
            candidates.Observe(@"C:\s\Meshes\seen_no.nif").Complete();

            var selected = candidates.Select([@"C:\s\Meshes\left_by_earlier_run.nif", @"C:\s\Meshes\seen_no.nif"]);

            Assert.Equal([@"C:\s\Meshes\left_by_earlier_run.nif"], selected);
        }

        [Fact]
        public void ANifWhoseWriteNeverCompletedIsACandidateWhateverItsPartialContentHeld()
        {
            var candidates = new CollisionCandidates();
            candidates.Observe(@"C:\s\Meshes\partial.nif").Write(NifWith("NiNode"));

            Assert.Equal([@"C:\s\Meshes\partial.nif"], candidates.Select([@"C:\s\Meshes\partial.nif"]));
        }

        [Fact]
        public void TheSelectionIsSortedByPathIgnoringCaseAndMatchesObservedPathsIgnoringCase()
        {
            var candidates = new CollisionCandidates();
            var withMarker = candidates.Observe(@"C:\S\MESHES\b\Two.NIF");
            withMarker.Write(NifWith("bhkPhysicsSystem"));
            withMarker.Complete();
            var without = candidates.Observe(@"c:\s\meshes\a\one.nif");
            without.Write(NifWith("NiNode"));
            without.Complete();

            var selected = candidates.Select([@"C:\s\Meshes\c\three.nif", @"C:\s\Meshes\b\two.nif", @"C:\s\Meshes\a\one.nif", @"C:\s\Meshes\Z.nif"]);

            Assert.Equal([@"C:\s\Meshes\b\two.nif", @"C:\s\Meshes\c\three.nif", @"C:\s\Meshes\Z.nif"], selected);
        }

        [Fact]
        public void ANifThatNoLongerExistsOnDiskIsNotListedEvenIfItHadTheMarker()
        {
            var candidates = new CollisionCandidates();
            var gone = candidates.Observe(@"C:\s\Meshes\gone.nif");
            gone.Write(Marker);
            gone.Complete();

            Assert.Empty(candidates.Select([]));
        }
    }
}
