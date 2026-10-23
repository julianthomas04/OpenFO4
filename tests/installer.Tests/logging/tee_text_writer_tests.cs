using AssetConverter.Logging;
using InstallerTests.Support;
using Xunit;

namespace InstallerTests.Logging
{
    public sealed class TeeTextWriterTests : IDisposable
    {
        private sealed class RecordingSink : ILineSink
        {
            private readonly List<(string Text, bool IsError)> _lines = new();

            public (string Text, bool IsError)[] Lines
            {
                get { lock (_lines) return [.. _lines]; }
            }

            public void WriteLine(string text, bool isError)
            {
                lock (_lines) _lines.Add((text, isError));
            }
        }

        private readonly StringWriter _inner = new();
        private readonly RecordingSink _sink = new();

        public void Dispose() => _inner.Dispose();

        private TeeTextWriter Tee(bool isError = false) => new(_inner, _sink, isError);

        [Fact]
        public void EverythingIsPassedThroughToTheOriginalWriterUnchanged()
        {
            var tee = Tee();

            tee.Write("partial ");
            tee.WriteLine("line");
            tee.Write('x');
            tee.WriteLine();
            tee.WriteLine("{0}-{1}", 1, "two");
            tee.Flush();

            Assert.Equal($"partial line{Environment.NewLine}x{Environment.NewLine}1-two{Environment.NewLine}", _inner.ToString());
        }

        [Fact]
        public void FlushWaitsUntilTheOriginalWriterHasTheTextAndFlushesIt()
        {
            var inner = new StallableTextWriter();
            var tee = new TeeTextWriter(inner, _sink, isError: false);

            tee.Write("pending text");
            tee.Flush();

            Assert.Equal("pending text", inner.ToString());
            Assert.True(inner.Flushes >= 1);
        }

        [Fact]
        public async Task AStalledConsoleDoesNotDelayTheSink()
        {
            var inner = new StallableTextWriter();
            var tee = new TeeTextWriter(inner, _sink, isError: false);
            inner.Stall();
            tee.WriteLine("blocks the console");
            inner.WaitUntilAWriteHasStarted();

            var burst = Task.Run(() =>
            {
                for (int i = 0; i < 200; i++) tee.WriteLine($"line {i}");
            }, TestContext.Current.CancellationToken);

            await burst.WaitAsync(TimeSpan.FromSeconds(30), TestContext.Current.CancellationToken);
            Assert.Equal(201, _sink.Lines.Length);
            Assert.Equal("line 199", _sink.Lines[^1].Text);
            inner.Release();
            tee.Dispose();
            Assert.StartsWith("blocks the console", inner.ToString(), StringComparison.Ordinal);
        }

        [Fact]
        public void AConsoleThatFailsIsReportedToTheSinkAsAnErrorAndTheSinkKeepsReceivingLines()
        {
            var tee = new TeeTextWriter(new BrokenConsole(), _sink, isError: false);

            tee.WriteLine("first");
            tee.Flush();
            tee.WriteLine("second");

            Assert.Contains(_sink.Lines, l => l.IsError && l.Text.Contains("console output is disabled", StringComparison.Ordinal));
            Assert.Contains(_sink.Lines, l => l.Text == "second");
        }

        [Fact]
        public void DisposingWhileTheConsoleIsStalledReportsThatItOnlyLivesInTheLogAfterTheTimeout()
        {
            var inner = new StallableTextWriter();
            var tee = new TeeTextWriter(inner, _sink, isError: false, stallTimeout: TimeSpan.FromMilliseconds(100));
            inner.Stall();
            tee.WriteLine("never shown");
            inner.WaitUntilAWriteHasStarted();

            tee.Dispose();
            inner.Release();

            Assert.Contains(_sink.Lines, l => l.IsError && l.Text.Contains("only in the log file", StringComparison.Ordinal));
        }

        [Fact]
        public void FlushingWhileTheConsoleIsStalledGivesUpAfterTheTimeoutAndReportsItOncePerStall()
        {
            var inner = new StallableTextWriter();
            var tee = new TeeTextWriter(inner, _sink, isError: false, stallTimeout: TimeSpan.FromMilliseconds(100));
            inner.Stall();
            tee.WriteLine("never shown");
            inner.WaitUntilAWriteHasStarted();

            tee.Flush();
            tee.Flush();
            inner.Release();
            tee.Dispose();

            Assert.Single(_sink.Lines, l => l.IsError && l.Text.Contains("flushing gave up waiting", StringComparison.Ordinal));
        }

        private sealed class BrokenConsole : StringWriter
        {
            public override void Write(string? value) => throw new IOException("handle is invalid");
        }

        [Fact]
        public void EachCompletedLineGoesToTheSinkWithoutItsLineEnding()
        {
            var tee = Tee();

            tee.WriteLine("first");
            tee.WriteLine("second");

            Assert.Equal([("first", false), ("second", false)], _sink.Lines);
        }

        [Fact]
        public void TextWithoutANewlineStaysPendingUntilTheLineCompletes()
        {
            var tee = Tee();

            tee.Write("Unpacking ");
            tee.Write("archive");
            Assert.Empty(_sink.Lines);
            tee.WriteLine("...");

            Assert.Equal([("Unpacking archive...", false)], _sink.Lines);
        }

        [Fact]
        public void AMultiLineWriteIsSplitIntoLines()
        {
            var tee = Tee();

            tee.Write("a\r\nb\nc");
            tee.Write("d\r\n");

            Assert.Equal([("a", false), ("b", false), ("cd", false)], _sink.Lines);
        }

        [Fact]
        public void EmptyLinesAreKept()
        {
            var tee = Tee();

            tee.WriteLine();
            tee.WriteLine("x");

            Assert.Equal([("", false), ("x", false)], _sink.Lines);
        }

        [Fact]
        public void TheErrorFlagIsPassedToTheSink()
        {
            var tee = Tee(isError: true);

            tee.WriteLine("boom");

            Assert.Equal([("boom", true)], _sink.Lines);
        }

        [Fact]
        public void DisposingFlushesAPendingPartialLineButNotTheOriginalWriter()
        {
            var tee = Tee();
            tee.Write("no newline at exit");

            tee.Dispose();

            Assert.Equal([("no newline at exit", false)], _sink.Lines);
            _inner.Write("still usable");
            Assert.EndsWith("still usable", _inner.ToString(), StringComparison.Ordinal);
        }

        [Fact]
        public void CharArrayAndSpanWritesAreCaptured()
        {
            var tee = Tee();

            tee.Write(['a', 'b', 'c'], 1, 2);
            tee.Write("de\n".AsSpan());
            tee.WriteLine("f".AsSpan());

            Assert.Equal([("bcde", false), ("f", false)], _sink.Lines);
        }

        [Fact]
        public void TheEncodingAndFormatProviderAreTheOriginalWritersOnes()
        {
            var tee = Tee();

            Assert.Same(_inner.Encoding, tee.Encoding);
            Assert.Same(_inner.FormatProvider, tee.FormatProvider);
        }

        [Fact]
        public void LinesWrittenFromManyThreadsStayIntact()
        {
            var tee = Tee();
            const int threads = 8;
            const int linesPerThread = 2000;

            Parallel.For(0, threads, new ParallelOptions { MaxDegreeOfParallelism = threads }, worker =>
            {
                for (int i = 0; i < linesPerThread; i++) tee.WriteLine($"worker {worker} line {i}");
            });

            var captured = _sink.Lines.Select(l => l.Text).ToHashSet();
            Assert.Equal(threads * linesPerThread, _sink.Lines.Length);
            for (int worker = 0; worker < threads; worker++)
                for (int i = 0; i < linesPerThread; i++)
                    Assert.Contains($"worker {worker} line {i}", captured);
        }
    }
}
