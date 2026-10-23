namespace AssetConverter.Pipeline
{
    /// <summary>
    /// Bytes granted by a <see cref="MemoryBudget"/>, released on disposal. The child processes doing the leased work are attached to it,
    /// so the budget counts their memory once: see <see cref="CoverChildProcesses"/>.
    /// </summary>
    public sealed class MemoryLease : IDisposable
    {
        private static readonly AsyncLocal<MemoryLease?> CoveringLease = new();

        private readonly MemoryBudget _budget;
        private volatile IChildMemory[] _children = Array.Empty<IChildMemory>();
        private int _released;

        internal MemoryLease(MemoryBudget budget, long bytes, bool longLived)
        {
            _budget = budget;
            Bytes = bytes;
            IsLongLived = longLived;
        }

        internal long Bytes { get; }

        internal bool IsLongLived { get; }

        internal bool IsReleased => Volatile.Read(ref _released) != 0;

        /// <summary>Written only under the budget's lock, replaced rather than changed, so it is read without one.</summary>
        internal IChildMemory[] Children
        {
            get => _children;
            set => _children = value;
        }

        /// <summary>
        /// Until the returned scope is disposed, every process started through <see cref="ToolProcess"/> on this flow, including the tasks it
        /// starts, is attached to this lease.
        /// </summary>
        public IDisposable CoverChildProcesses()
        {
            var scope = new CoverScope(CoveringLease.Value);
            CoveringLease.Value = this;
            return scope;
        }

        /// <summary>Attaches to the lease covering the current flow, if any, the child memory <paramref name="open"/> opens for the footprint its budget's gauge counts.</summary>
        internal static void AttachToCovering(Func<ChildFootprint, IChildMemory> open)
        {
            if (CoveringLease.Value is { } lease && lease._budget.ChildFootprint != ChildFootprint.NotCounted)
                lease.Attach(open(lease._budget.ChildFootprint));
        }

        /// <summary>The lease takes ownership of <paramref name="child"/>.</summary>
        internal void Attach(IChildMemory child) => _budget.Attach(this, child);

        /// <summary>The bytes of the lease its attached processes have committed; none when the memory of one of them cannot be read.</summary>
        internal long CommittedChildBytes(out bool unreadable)
        {
            unreadable = false;
            long committed = 0;
            foreach (var child in _children)
            {
                if (child.ReadBytes() is not { } bytes)
                {
                    unreadable = true;
                    return 0;
                }

                committed += bytes;
            }

            return Math.Min(Bytes, committed);
        }

        public void Dispose()
        {
            if (Interlocked.Exchange(ref _released, 1) == 0)
                _budget.Release(this);
        }

        private sealed class CoverScope(MemoryLease? previous) : IDisposable
        {
            public void Dispose() => CoveringLease.Value = previous;
        }
    }
}
