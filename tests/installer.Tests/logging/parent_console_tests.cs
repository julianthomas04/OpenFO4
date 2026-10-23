using AssetConverter.Logging;
using Xunit;

namespace InstallerTests.Logging
{
    public sealed class ParentConsoleTests
    {
        private sealed class FakeConsoleHost(bool hasParentConsole, params StandardStream[] redirected) : IConsoleHost
        {
            public Dictionary<StandardStream, StringWriter> Consoles { get; } = [];

            public List<string> Calls { get; } = [];

            public bool AttachToParent()
            {
                Calls.Add("attach");
                return hasParentConsole;
            }

            public bool IsRedirected(StandardStream stream)
            {
                Calls.Add($"redirected?{stream}");
                return redirected.Contains(stream);
            }

            public TextWriter OpenConsoleWriter(StandardStream stream)
            {
                Calls.Add($"open {stream}");
                return Consoles[stream] = new StringWriter();
            }
        }

        private static (bool Attached, Dictionary<StandardStream, TextWriter> Bound) Run(FakeConsoleHost host)
        {
            var bound = new Dictionary<StandardStream, TextWriter>();
            bool attached = ParentConsole.Attach(host, (stream, writer) => bound[stream] = writer);
            return (attached, bound);
        }

        [Fact]
        public void StartedFromATerminalTheOutputAndErrorStreamsGoToItsConsole()
        {
            var host = new FakeConsoleHost(hasParentConsole: true);

            var (attached, bound) = Run(host);

            Assert.True(attached);
            Assert.Same(host.Consoles[StandardStream.Output], bound[StandardStream.Output]);
            Assert.Same(host.Consoles[StandardStream.Error], bound[StandardStream.Error]);
        }

        [Fact]
        public void StartedWithoutATerminalNothingIsRebound()
        {
            var host = new FakeConsoleHost(hasParentConsole: false);

            var (attached, bound) = Run(host);

            Assert.False(attached);
            Assert.Empty(bound);
            Assert.DoesNotContain(host.Calls, call => call.StartsWith("open", StringComparison.Ordinal));
        }

        [Fact]
        public void AStreamTheStarterRedirectedKeepsItsRedirectionWhileTheOtherOneGoesToTheTerminal()
        {
            var host = new FakeConsoleHost(hasParentConsole: true, StandardStream.Output);

            var (attached, bound) = Run(host);

            Assert.True(attached);
            Assert.Equal([StandardStream.Error], bound.Keys);
        }

        [Fact]
        public void TheRedirectionIsReadBeforeAttachingBecauseAttachingMayGiveEveryStreamAHandle()
        {
            var host = new FakeConsoleHost(hasParentConsole: true);

            Run(host);

            Assert.True(host.Calls.IndexOf("redirected?Output") < host.Calls.IndexOf("attach"));
            Assert.True(host.Calls.IndexOf("redirected?Error") < host.Calls.IndexOf("attach"));
        }
    }
}
