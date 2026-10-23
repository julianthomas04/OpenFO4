using AssetConverter;
using AssetConverter.Swf;
using Xunit;

namespace InstallerTests
{
    // The shared parts of the interface conversion steps: atomic publishing, failure handling and the ffdec wrapper.
    public sealed class ConversionInfrastructureTests : IDisposable
    {
        private readonly TempDirectory _root = new();

        public void Dispose() => _root.Dispose();

        [Fact]
        public void PublishFile_WritesTheContentAndLeavesNoTemporaryFile()
        {
            string path = Path.Combine(_root.Path, "below", "out.json");

            AtomicPublish.File(path, "{}");
            AtomicPublish.File(path, "[1]");

            Assert.Equal("[1]", File.ReadAllText(path));
            Assert.False(File.Exists(path + AtomicPublish.TemporarySuffix));
        }

        [Fact]
        public void PublishFolder_ReplacesThePreviousOutputWithAWholeNewOne()
        {
            string folder = Path.Combine(_root.Path, "art");
            Directory.CreateDirectory(folder);
            File.WriteAllText(Path.Combine(folder, "old.txt"), "old");

            AtomicPublish.Folder(folder, temporary => File.WriteAllText(Path.Combine(temporary, "new.txt"), "new"));

            Assert.Equal(["new.txt"], Directory.GetFiles(folder).Select(Path.GetFileName));
            Assert.Equal([folder], Directory.GetDirectories(_root.Path));
        }

        [Fact]
        public void PublishFolder_FailingWhileBuilding_KeepsThePreviousOutput()
        {
            string folder = Path.Combine(_root.Path, "art");
            Directory.CreateDirectory(folder);
            File.WriteAllText(Path.Combine(folder, "old.txt"), "old");

            Assert.Throws<InvalidOperationException>(() => AtomicPublish.Folder(folder, temporary =>
            {
                File.WriteAllText(Path.Combine(temporary, "half.txt"), "half");
                throw new InvalidOperationException("boom");
            }));

            Assert.Equal(["old.txt"], Directory.GetFiles(folder).Select(Path.GetFileName));
            Assert.Equal([folder], Directory.GetDirectories(_root.Path));
        }

        [Fact]
        public void PublishFolder_NoPreviousOutput_CreatesIt()
        {
            string folder = Path.Combine(_root.Path, "fresh");

            AtomicPublish.Folder(folder, temporary => File.WriteAllText(Path.Combine(temporary, "a.txt"), "a"));

            Assert.True(File.Exists(Path.Combine(folder, "a.txt")));
        }

        private static Exception Step(string message, Exception cause) => new PipboyArtException(message, cause);

        [Fact]
        public void Run_MalformedData_NamesTheSubjectAndKeepsTheCause()
        {
            var cause = new SwfFormatException("bad tag");

            var failure = Assert.Throws<PipboyArtException>(() => ConversionGuard.Run<int>("Sample.swf", () => throw cause, Step));

            Assert.Equal("'Sample.swf' cannot be converted: bad tag", failure.Message);
            Assert.Same(cause, failure.InnerException);
        }

        [Fact]
        public void Run_FfdecFailure_IsReportedAsConversionFailureOfTheSubject()
        {
            var failure = Assert.Throws<PipboyArtException>(() =>
                ConversionGuard.Run<int>("Sample.swf", () => throw new FfdecException("exited 1"), Step));

            Assert.Contains("Sample.swf", failure.Message);
            Assert.Contains("exited 1", failure.Message);
        }

        [Fact]
        public void Run_RuntimeFault_NamesTheSubjectAndTheFaultType()
        {
            var failure = Assert.Throws<PipboyArtException>(() =>
                ConversionGuard.Run<int>("Sample.swf", () => throw new InvalidCastException("odd"), Step));

            Assert.Contains("Sample.swf", failure.Message);
            Assert.Contains(nameof(InvalidCastException), failure.Message);
        }

        [Fact]
        public void Run_NotAnInputFault_IsNotWrapped()
        {
            Assert.Throws<NotSupportedException>(() => ConversionGuard.Run<int>("Sample.swf", () => throw new NotSupportedException(), Step));
        }

        [Fact]
        public void RunAll_EveryJobRunsAndEveryFailureIsReportedInMessageOrder()
        {
            int ran = 0;
            Action[] jobs =
            [
                () => { Interlocked.Increment(ref ran); throw new InvalidOperationException("b failed"); },
                () => Interlocked.Increment(ref ran),
                () => { Interlocked.Increment(ref ran); throw new InvalidOperationException("a failed"); },
            ];

            var failure = Assert.Throws<PipboyArtException>(() => ConversionGuard.RunAll(jobs, 2,
                failures => new PipboyArtException(ConversionGuard.Describe(failures), failures[0])));

            Assert.Equal(3, ran);
            Assert.Equal("a failed b failed", failure.Message);
        }

        [Fact]
        public void RunAll_NoFailure_ReturnsNormally()
        {
            ConversionGuard.RunAll([() => { }, () => { }], 2, failures => new PipboyArtException("unexpected"));
        }

        [Theory]
        [InlineData("JPEXS Free Flash Decompiler v.26.2.1\nUsage: ...", "26.2.1")]
        [InlineData("\r\n  JPEXS Free Flash Decompiler v.1.0.0-beta2\r\nmore", "1.0.0-beta2")]
        public void ParseVersion_ReadsTheVersionTheHelpOutputOpensWith(string help, string expected)
        {
            Assert.Equal(expected, Ffdec.ParseVersion(help));
        }

        [Theory]
        [InlineData("")]
        [InlineData("Usage: something else")]
        [InlineData("JPEXS Free Flash Decompiler\n")]
        public void ParseVersion_OutputWithoutAVersion_Throws(string help)
        {
            Assert.Throws<FfdecException>(() => Ffdec.ParseVersion(help));
        }
    }
}
