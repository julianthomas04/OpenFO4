using AssetConverter;
using Xunit;

namespace InstallerTests
{
    public class PriorityFilesTests
    {
        private static readonly string Root = Path.Combine(Path.GetTempPath(), "priority-files");

        private static string Nif(string name) => Path.Combine(Root, name);

        private static string[] Corpus(int count) => [.. Enumerable.Range(0, count).Select(i => Nif($"mesh{i:D3}.nif"))];

        private static PriorityFiles Prioritizing(Action allDone, params string[] paths)
        {
            var priority = new PriorityFiles(allDone);
            priority.Prioritize(paths);
            return priority;
        }

        [Fact]
        public void Wants_MatchesPathsIgnoringCaseAndSpelling()
        {
            var priority = Prioritizing(() => { }, Path.Combine(Root, "x", "..", "MESH002.NIF"));

            Assert.True(priority.Wants(Nif("mesh002.nif")));
            Assert.False(priority.Wants(Nif("mesh003.nif")));
        }

        [Fact]
        public void Wants_WithoutAPriorityList_IsFalseForEveryFile()
        {
            Assert.False(new PriorityFiles(() => { }).Wants(Nif("mesh000.nif")));
        }

        [Fact]
        public void TheSignalFiresAfterExactlyTheLastPriorityFile_NotBeforeAndNotAfterOtherFiles()
        {
            var files = Corpus(6);
            int signals = 0;
            var priority = Prioritizing(() => signals++, files[1], files[4], files[5]);
            priority.Expect(files);

            priority.Converted(files[1]);
            priority.Converted(files[4]);
            Assert.Equal(0, signals);
            priority.Converted(files[5]);
            Assert.Equal(1, signals);
            foreach (string other in files) priority.Converted(other);
            Assert.Equal(1, signals);
        }

        [Fact]
        public void AFileThatIsNotPriorityDoesNotCountTowardsTheSignal()
        {
            var files = Corpus(4);
            int signals = 0;
            var priority = Prioritizing(() => signals++, files[2]);
            priority.Expect(files);

            priority.Converted(files[0]);
            priority.Converted(files[3]);

            Assert.Equal(0, signals);
        }

        [Fact]
        public void APriorityFileCountsOnlyOnce()
        {
            var files = Corpus(3);
            int signals = 0;
            var priority = Prioritizing(() => signals++, files[0], files[1]);
            priority.Expect(files);

            priority.Converted(files[0]);
            priority.Converted(files[0]);

            Assert.Equal(0, signals);
        }

        [Fact]
        public void APriorityFileThatIsNotPartOfTheConversionIsNotWaitedFor()
        {
            var files = Corpus(3);
            int signals = 0;
            var priority = Prioritizing(() => signals++, files[1], Nif("elsewhere.nif"));
            priority.Expect(files);

            priority.Converted(files[1]);

            Assert.Equal(1, signals);
        }

        [Fact]
        public void WithoutAnyPriorityFileInTheConversion_TheSignalFiresAtOnce()
        {
            int signals = 0;
            var priority = Prioritizing(() => signals++, Nif("elsewhere.nif"));

            priority.Expect(Corpus(3));

            Assert.Equal(1, signals);
        }

        [Fact]
        public void AnEmptyConversionSignalsAtOnce()
        {
            int signals = 0;

            Prioritizing(() => signals++, Nif("a.nif")).Expect([]);

            Assert.Equal(1, signals);
        }

        [Fact]
        public void Release_SignalsForAPriorityFileThatNeverArrivedAndOnlyOnce()
        {
            var files = Corpus(3);
            int signals = 0;
            var priority = Prioritizing(() => signals++, files[0], files[1]);
            priority.Expect(files);
            priority.Converted(files[0]);

            priority.Release();
            priority.Release();
            priority.Converted(files[1]);

            Assert.Equal(1, signals);
        }

        [Fact]
        public void ConvertedFromManyThreads_SignalsExactlyOnce()
        {
            var files = Corpus(2000);
            int signals = 0;
            var priority = Prioritizing(() => Interlocked.Increment(ref signals), [.. files.Take(500)]);
            priority.Expect(files);

            Parallel.ForEach(files, new ParallelOptions { MaxDegreeOfParallelism = 8 }, priority.Converted);

            Assert.Equal(1, signals);
        }
    }
}
