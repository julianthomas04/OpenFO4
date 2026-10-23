using System.IO.Pipes;
using AssetConverter.Logging;
using Xunit;

namespace InstallerTests.Logging
{
    public sealed class LogFileWriterTests : IDisposable
    {
        private readonly string _directory = Directory.CreateTempSubdirectory("installer-tests-log-").FullName;

        public void Dispose() => Directory.Delete(_directory, recursive: true);

        private string LogPath => Path.Combine(_directory, "install.log");

        private static string[] ReadWhileOpen(string path)
        {
            using var stream = new FileStream(path, FileMode.Open, FileAccess.Read, FileShare.ReadWrite);
            using var reader = new StreamReader(stream);
            return reader.ReadToEnd().Split('\n', StringSplitOptions.RemoveEmptyEntries | StringSplitOptions.TrimEntries);
        }

        [Fact]
        public void EveryLineIsOnDiskImmediatelySoACrashKeepsTheLog()
        {
            using var writer = new LogFileWriter(LogPath, _ => { });

            writer.WriteLine("first", isError: false);
            string[] afterFirst = ReadWhileOpen(LogPath);
            writer.WriteLine("second", isError: true);
            string[] afterSecond = ReadWhileOpen(LogPath);

            Assert.Single(afterFirst);
            Assert.EndsWith("first", afterFirst[0], StringComparison.Ordinal);
            Assert.Equal(2, afterSecond.Length);
            Assert.EndsWith(" ERR second", afterSecond[1], StringComparison.Ordinal);
        }

        [Fact]
        public void ARunReplacesTheLogOfThePreviousRun()
        {
            using (var previous = new LogFileWriter(LogPath, _ => { }))
            {
                previous.WriteLine("old run", false);
            }

            using var current = new LogFileWriter(LogPath, _ => { });
            current.WriteLine("new run", false);

            string[] lines = ReadWhileOpen(LogPath);
            Assert.Single(lines);
            Assert.EndsWith("new run", lines[0], StringComparison.Ordinal);
        }

        [Fact]
        public void MissingDirectoriesAreCreated()
        {
            string nested = Path.Combine(_directory, "a", "b", "install.log");

            using var writer = new LogFileWriter(nested, _ => { });
            writer.WriteLine("x", false);

            Assert.True(File.Exists(nested));
            Assert.Equal(nested, writer.Path);
        }

        [Fact]
        public void TheFileIsPlainUtf8WithoutAByteOrderMark()
        {
            using (var writer = new LogFileWriter(LogPath, _ => { }))
            {
                writer.WriteLine("— ünïcode", false);
            }

            byte[] bytes = File.ReadAllBytes(LogPath);
            Assert.NotEqual(0xEF, bytes[0]);
            Assert.Contains("— ünïcode", File.ReadAllText(LogPath), StringComparison.Ordinal);
        }

        /// <summary>A pipe whose reader has gone away: every write to it fails with an <see cref="IOException"/>, like a disk that went offline.</summary>
        private static AnonymousPipeServerStream BrokenStream()
        {
            var server = new AnonymousPipeServerStream(PipeDirection.Out);
            using (var client = new AnonymousPipeClientStream(PipeDirection.In, server.ClientSafePipeHandle))
            {
                server.DisposeLocalCopyOfClientHandle();
            }
            return server;
        }

        [Fact]
        public void AFailingDiskDisablesTheFileAndIsReportedOnceWithoutThrowing()
        {
            var faults = new List<string>();
            using var stream = BrokenStream();
            using var writer = new LogFileWriter(stream, LogPath, faults.Add);

            writer.WriteLine("first", false);
            writer.WriteLine("second", false);
            writer.WriteLine("third", true);

            string fault = Assert.Single(faults);
            Assert.Contains(LogPath, fault, StringComparison.Ordinal);
            Assert.Contains("disabled", fault, StringComparison.Ordinal);
        }

        [Fact]
        public void DisposingAFailedWriterDoesNotThrowAndDoesNotReportTwice()
        {
            var faults = new List<string>();
            using var stream = BrokenStream();
            var writer = new LogFileWriter(stream, LogPath, faults.Add);
            writer.WriteLine("first", false);

            writer.Dispose();

            Assert.Single(faults);
        }

        [Fact]
        public void LinesWrittenAfterDisposeAreIgnoredWithoutAnError()
        {
            var faults = new List<string>();
            var writer = new LogFileWriter(LogPath, faults.Add);
            writer.WriteLine("kept", false);
            writer.Dispose();

            writer.WriteLine("too late", false);
            writer.Dispose();

            Assert.Empty(faults);
            string[] lines = File.ReadAllLines(LogPath);
            Assert.Single(lines);
            Assert.EndsWith("kept", lines[0], StringComparison.Ordinal);
        }

        [Fact]
        public void ALogFileHeldByAnotherInstallerCannotBeOpened()
        {
            using var first = new LogFileWriter(LogPath, _ => { });

            Assert.Throws<IOException>(() => new LogFileWriter(LogPath, _ => { }));
        }

        [Fact]
        public void ConcurrentWritersProduceWholeLines()
        {
            using (var writer = new LogFileWriter(LogPath, _ => { }))
            {
                Parallel.For(0, 8, worker =>
                {
                    for (int i = 0; i < 500; i++) writer.WriteLine($"worker {worker} line {i}", false);
                });
            }

            string[] lines = File.ReadAllLines(LogPath);
            Assert.Equal(4000, lines.Length);
            Assert.All(lines, line => Assert.Matches(@"^\d\d:\d\d:\d\d\.\d{3}\s+worker \d line \d+$", line));
        }
    }
}
