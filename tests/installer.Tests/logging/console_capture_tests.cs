using AssetConverter.Logging;
using Xunit;

namespace InstallerTests.Logging
{
    /// <summary>Redirects the process-wide console, so these tests never run in parallel with each other.</summary>
    [Collection("Console redirection")]
    public class ConsoleCaptureTests
    {
        [Fact]
        public void ConsoleOutputAndErrorReachTheSinkAndTheOriginalWritersThenTheOriginalsAreRestored()
        {
            var originalOut = Console.Out;
            var originalError = Console.Error;
            var fakeOut = new StringWriter();
            var fakeError = new StringWriter();
            Console.SetOut(fakeOut);
            Console.SetError(fakeError);
            var buffer = new LogBuffer(100);
            try
            {
                using (ConsoleCapture.Start(buffer))
                {
                    Console.WriteLine("hello from out");
                    Console.Error.WriteLine("hello from error");
                    Console.Write("unterminated");
                }

                var lines = buffer.ReadAfter(0).Lines;
                Assert.Contains(lines, l => l.Text == "hello from out" && !l.IsError);
                Assert.Contains(lines, l => l.Text == "hello from error" && l.IsError);
                Assert.Contains(lines, l => l.Text == "unterminated");
                Assert.Contains("hello from out", fakeOut.ToString(), StringComparison.Ordinal);
                Assert.Contains("hello from error", fakeError.ToString(), StringComparison.Ordinal);

                Console.WriteLine("after capture");
                Assert.DoesNotContain(buffer.ReadAfter(0).Lines, l => l.Text == "after capture");
                Assert.Contains("after capture", fakeOut.ToString(), StringComparison.Ordinal);
            }
            finally
            {
                Console.SetOut(originalOut);
                Console.SetError(originalError);
            }
        }

        [Fact]
        public void DisposingTwiceIsHarmless()
        {
            var originalOut = Console.Out;
            var originalError = Console.Error;
            try
            {
                var capture = ConsoleCapture.Start(new LogBuffer(10));
                capture.Dispose();
                capture.Dispose();

                Assert.Same(originalOut, Console.Out);
                Assert.Same(originalError, Console.Error);
            }
            finally
            {
                Console.SetOut(originalOut);
                Console.SetError(originalError);
            }
        }
    }
}
