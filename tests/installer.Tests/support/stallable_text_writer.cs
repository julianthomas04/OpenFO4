using Xunit;

namespace InstallerTests.Support
{
    /// <summary>A string writer whose writes block while stalled, like a console in selection mode.</summary>
    internal sealed class StallableTextWriter : StringWriter
    {
        private static readonly TimeSpan Patience = TimeSpan.FromSeconds(30);

        private readonly ManualResetEventSlim _open = new(initialState: true);
        private readonly ManualResetEventSlim _entered = new(initialState: false);
        private int _flushes;

        public int Flushes => Volatile.Read(ref _flushes);

        public void Stall() => _open.Reset();

        public void Release() => _open.Set();

        /// <summary>Blocks until a write has started, which while stalled means the writer thread is now stuck inside it.</summary>
        public void WaitUntilAWriteHasStarted() => Assert.True(_entered.Wait(Patience), "no write reached the stalled writer");

        public override void Write(string? value)
        {
            _entered.Set();
            Assert.True(_open.Wait(Patience), "the stalled writer was never released");
            base.Write(value);
        }

        public override void Flush()
        {
            Interlocked.Increment(ref _flushes);
            base.Flush();
        }
    }
}
