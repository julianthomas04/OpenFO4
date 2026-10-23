namespace AssetConverter.Pipeline
{
    /// <summary>
    /// Memory-aware admission for the install's parallel work: a unit reserves its estimated byte cost before it starts and releases it
    /// by disposing the lease, and a unit that does not fit waits. A request that fits while nobody waits is granted at once, without
    /// queueing. Otherwise admission is first-fit in arrival order: on every release or cancellation the waiters are scanned oldest
    /// first and each one that fits is granted, and a new request that fits is granted even while older, larger ones wait.
    /// </summary>
    /// <remarks>
    /// <para>
    /// First-fit alone lets small work starve a large request forever, so a waiter that has waited longer than
    /// <see cref="AgingThreshold"/> becomes a barrier: no later waiter is granted until it is.
    /// </para>
    /// <para>
    /// The estimates cannot see memory that is held but not leased: the installer's own baseline, the runtime's free heap, child
    /// processes, other programs. A gauge of the memory actually in use backs them up: a request is granted only when the reserved bytes
    /// plus the measured bytes plus the request fit the capacity. Leases that have already materialized are counted in both terms, which
    /// is deliberately conservative: a lease that has not materialized yet would otherwise hide the baseline. Child processes are the
    /// exception, since their memory can be read directly: when the gauge counts them, the memory of the processes attached to a lease,
    /// up to its bytes, is taken off the measured bytes, so the lease counts only what they have not committed yet. That memory is read
    /// only when the gauge alone holds a request back, and before the gauge, so memory committed in between counts twice rather than
    /// not at all. A process that has exited counts nothing; one whose memory cannot be read leaves its lease counted twice, logged
    /// once, and so do all of them while the gauge yields a stale reading or when the leases with children change during the read. While a request is held
    /// back only by the measurement, the budget re-checks the gauge periodically, since nothing is released that would wake the
    /// waiters, logs it (at most once per <see cref="LogInterval"/>) and compacts the GC heap, but only when the runtime holds enough
    /// unused memory for that to help and at most once per <see cref="ReclaimInterval"/>. A gauge that fails after it has worked
    /// keeps its last reading, logged once, so a transient failure does not fail the install; a gauge that never worked throws.
    /// </para>
    /// <para>
    /// Progress guarantee: when no transient lease is held, the heap is reclaimed (when due and worthwhile), the gauge read again, and
    /// a waiter that still does not fit is granted anyway, one per pass, so memory the install does not control cannot stall it. A
    /// request is clamped to what the long-lived leases leave of the capacity, so an oversized request runs alone among the transient work.
    /// </para>
    /// <para>
    /// Long-lived leases (<see cref="ReserveLongLived"/>) are held across a wait on other work, such as a worker pool waiting for
    /// streamed input, and do not count against the progress guarantee. Callers keep two rules: a flow never waits for a lease while
    /// holding a transient one, and a long-lived lease never waits for anything that itself needs a long-lived lease and the total
    /// of the long-lived leases stays well below the capacity, so the work it waits on always fits beside it.
    /// </para>
    /// </remarks>
    public sealed class MemoryBudget
    {
        /// <summary>The share of the effective memory limit the install may reserve, leaving the rest for what no estimate covers.</summary>
        internal const double CapacityFraction = 0.8;

        /// <summary>The least unused runtime memory for which a GC reclamation is worth its pause.</summary>
        internal const long MinimumReclaimableBytes = 64L << 20;

        /// <summary>How often the gauge is re-read while a request is held back only by the measurement.</summary>
        internal static readonly TimeSpan RecheckInterval = TimeSpan.FromMilliseconds(250);

        /// <summary>The shortest time between two log lines about waiting, so long blocks stay visible without flooding the log.</summary>
        internal static readonly TimeSpan LogInterval = TimeSpan.FromSeconds(10);

        /// <summary>The shortest time between two looks at whether the GC heap can be reclaimed.</summary>
        internal static readonly TimeSpan ReclaimInterval = TimeSpan.FromSeconds(1);

        /// <summary>How long the oldest waiter waits before it stops later waiters from being granted ahead of it.</summary>
        internal static readonly TimeSpan AgingThreshold = TimeSpan.FromSeconds(30);

        private readonly object _lock = new();
        private readonly LinkedList<Waiter> _waiters = new();
        private volatile MemoryLease[] _leasesWithChildren = [];
        private Int128 _reservedBytes;
        private Int128 _longLivedBytes;
        private ITimer? _recheckTimer;
        private long? _lastReclaimTimestamp;
        private long? _lastLogTimestamp;
        private long _lastGoodMeasurement = -1;
        private int _gaugeFailing;
        private int _childReadFailed;

        public MemoryBudget(long capacityBytes)
        {
            ArgumentOutOfRangeException.ThrowIfNegativeOrZero(capacityBytes);
            Capacity = capacityBytes;
        }

        public long Capacity { get; }

        /// <summary>The effective memory limit <see cref="Capacity"/> was derived from, when there is one.</summary>
        public long? LimitBytes { get; init; }

        /// <summary>Reads the memory currently used of the pool the limit constrains. Without one, only the reservations count.</summary>
        internal Func<long> UsedBytes { get; init; } = () => 0;

        /// <summary>Which memory of the installer's child processes <see cref="UsedBytes"/> counts, which their leases then need not count again.</summary>
        internal ChildFootprint ChildFootprint { get; init; }

        internal TimeProvider Clock { get; init; } = TimeProvider.System;

        /// <summary>Returns unused memory held by the runtime to the system.</summary>
        internal Action Reclaim { get; init; } = ReclaimGcMemory;

        /// <summary>Estimates the memory a <see cref="Reclaim"/> could return, to skip it when it would not help.</summary>
        internal Func<long> ReclaimableBytes { get; init; } = EstimateGcReclaimableBytes;

        internal Action<string> Log { get; init; } = Console.WriteLine;

        public long ReservedBytes
        {
            get { lock (_lock) return SaturateToLong(_reservedBytes); }
        }

        public int WaitingCount
        {
            get { lock (_lock) return _waiters.Count; }
        }

        /// <summary>
        /// A budget sized from the effective memory limit, which is the smaller of the physical memory and any Job Object memory limit.
        /// </summary>
        public static MemoryBudget FromSystemLimit()
        {
            var limit = SystemMemoryLimit.Query(Console.WriteLine);
            return FromLimit(limit.Bytes, limit.UsedBytes, limit.ChildFootprint);
        }

        internal static MemoryBudget FromLimit(long limitBytes, Func<long>? usedBytes = null, ChildFootprint childFootprint = ChildFootprint.NotCounted) =>
            new((long)(limitBytes * CapacityFraction))
            {
                LimitBytes = limitBytes,
                UsedBytes = usedBytes ?? (() => 0),
                ChildFootprint = usedBytes is null ? ChildFootprint.NotCounted : childFootprint,
            };

        /// <summary>A budget every request fits in at once, for callers that run without memory admission.</summary>
        public static MemoryBudget Unlimited() => new(long.MaxValue);

        public string Describe() =>
            LimitBytes is { } limit
                ? $"Memory budget: {ByteFormat.Format(Capacity)} of {ByteFormat.Format(limit)} limit"
                : $"Memory budget: {ByteFormat.Format(Capacity)}";

        /// <summary>Blocks the calling thread until the bytes fit. Throws <see cref="OperationCanceledException"/> when cancelled while waiting.</summary>
        public MemoryLease Reserve(long bytes, CancellationToken ct) => Reserve(bytes, LeaseLife.Transient, ct);

        /// <summary>
        /// Like <see cref="Reserve"/>, for a lease held across a wait on other work. It does not count as held work for the progress
        /// guarantee and shrinks what every request is clamped to.
        /// </summary>
        public MemoryLease ReserveLongLived(long bytes, CancellationToken ct) => Reserve(bytes, LeaseLife.LongLived, ct);

        /// <summary>Completes when the bytes fit. Faults with <see cref="OperationCanceledException"/> when cancelled while waiting.</summary>
        public Task<MemoryLease> ReserveAsync(long bytes, CancellationToken ct)
        {
            if (TryReserveImmediately(bytes, LeaseLife.Transient, ct, out Measurement measured) is { } lease)
                return Task.FromResult(lease);
            return EnqueueAndWait(new Waiter(bytes, LeaseLife.Transient), measured, ct);
        }

        private MemoryLease Reserve(long bytes, LeaseLife life, CancellationToken ct)
        {
            if (TryReserveImmediately(bytes, life, ct, out Measurement measured) is { } lease)
                return lease;
            return EnqueueAndWait(new Waiter(bytes, life), measured, ct).GetAwaiter().GetResult();
        }

        /// <summary>
        /// The fast path: with nobody waiting and the request fitting, the lease is granted without a waiter. The gauge is read before the
        /// lock, and the child processes only when the gauge alone holds the request back.
        /// </summary>
        private MemoryLease? TryReserveImmediately(long bytes, LeaseLife life, CancellationToken ct, out Measurement measured)
        {
            ArgumentOutOfRangeException.ThrowIfNegativeOrZero(bytes);
            ct.ThrowIfCancellationRequested();
            measured = Measure();
            var lease = TryTake(bytes, life, measured.UsedBytes, out bool othersWait);
            if (lease is not null || othersWait || measured.ChildrenAccountedFor)
                return lease;

            measured = MeasureBeyondLeases();
            return TryTake(bytes, life, measured.UsedBytes, out _);
        }

        private MemoryLease? TryTake(long bytes, LeaseLife life, long measuredBytes, out bool othersWait)
        {
            lock (_lock)
            {
                othersWait = _waiters.Count > 0;
                if (othersWait)
                    return null;
                long granted = ClampToRoom(bytes);
                return Fits(granted, measuredBytes) ? Take(granted, life) : null;
            }
        }

        private async Task<MemoryLease> EnqueueAndWait(Waiter waiter, Measurement measured, CancellationToken ct)
        {
            lock (_lock)
            {
                waiter.EnqueuedTimestamp = Clock.GetTimestamp();
                waiter.Node = _waiters.AddLast(waiter);
            }

            Evaluate(measured);
            using var cancellation = ct.Register(() => Abandon(waiter, ct));
            return await waiter.Completion.Task.ConfigureAwait(false);
        }

        private void Abandon(Waiter waiter, CancellationToken ct)
        {
            lock (_lock)
            {
                if (waiter.Node.List is null)
                    return;
                _waiters.Remove(waiter.Node);
                waiter.Completion.TrySetCanceled(ct);
            }

            Reevaluate();
        }

        internal void Release(MemoryLease lease)
        {
            bool waiting;
            IChildMemory[] children;
            lock (_lock)
            {
                _reservedBytes -= lease.Bytes;
                if (lease.IsLongLived)
                    _longLivedBytes -= lease.Bytes;
                children = lease.Children;
                if (children.Length > 0)
                    _leasesWithChildren = Without(_leasesWithChildren, lease);
                waiting = _waiters.Count > 0;
                if (!waiting)
                    EndBlockedEpisode();
            }

            foreach (var child in children)
                child.Dispose();
            if (waiting)
                Reevaluate();
        }

        /// <summary>A child attached to a lease that is already released is disposed at once.</summary>
        internal void Attach(MemoryLease lease, IChildMemory child)
        {
            lock (_lock)
            {
                if (!lease.IsReleased)
                {
                    if (lease.Children.Length == 0)
                        _leasesWithChildren = [.. _leasesWithChildren, lease];
                    lease.Children = [.. lease.Children, child];
                    return;
                }
            }

            child.Dispose();
        }

        /// <summary>Reads the gauge and grants the waiters that fit now.</summary>
        private void Reevaluate()
        {
            if (HasWaiters())
                Evaluate(Measure());
        }

        /// <summary>Whether waiters remain; with none left the blocked episode is over and its timer stops.</summary>
        private bool HasWaiters()
        {
            lock (_lock)
            {
                if (_waiters.Count == 0)
                    EndBlockedEpisode();
                return _waiters.Count > 0;
            }
        }

        /// <summary>The gauge alone, which leaves the child processes' commit to be taken off when the gauge holds a request back.</summary>
        private Measurement Measure()
        {
            long used = ReadUsed(out bool fresh);
            return new Measurement(used, ChildrenAccountedFor: !fresh || _leasesWithChildren.Length == 0);
        }

        /// <summary>
        /// The measured bytes without what the leases' child processes have committed of them, which the reserved bytes already count. A
        /// stale gauge reading, or a change to the leases with children while they are read, takes nothing off.
        /// </summary>
        private Measurement MeasureBeyondLeases()
        {
            var leases = _leasesWithChildren;
            long committed = ReadCommittedChildBytes(leases);
            long used = ReadUsed(out bool fresh);
            bool leasesUnchanged = ReferenceEquals(leases, _leasesWithChildren);
            return new Measurement(fresh && leasesUnchanged ? Math.Max(0, used - committed) : used, ChildrenAccountedFor: true);
        }

        private long ReadCommittedChildBytes(MemoryLease[] leases)
        {
            long committed = 0;
            foreach (var lease in leases)
            {
                committed += lease.CommittedChildBytes(out bool unreadable);
                if (unreadable && Interlocked.Exchange(ref _childReadFailed, 1) == 0)
                    Log("Memory: the memory of a child process could not be read; its lease counts in full beside it");
            }

            return committed;
        }

        /// <summary>
        /// Reads the gauge. After a first successful read a failing gauge yields the last good reading, which is not
        /// <paramref name="fresh"/>, logged once per failure streak; before one, the failure is the caller's to see.
        /// </summary>
        private long ReadUsed(out bool fresh)
        {
            try
            {
                long used = Math.Max(0, UsedBytes());
                if (Volatile.Read(ref _lastGoodMeasurement) != used)
                    Volatile.Write(ref _lastGoodMeasurement, used);
                if (Volatile.Read(ref _gaugeFailing) != 0)
                    Volatile.Write(ref _gaugeFailing, 0);
                fresh = true;
                return used;
            }
            catch (Exception exception) when (Volatile.Read(ref _lastGoodMeasurement) >= 0)
            {
                long last = Volatile.Read(ref _lastGoodMeasurement);
                if (Interlocked.Exchange(ref _gaugeFailing, 1) == 0)
                    Log($"Memory: reading the memory in use failed ({exception.GetType().Name}: {exception.Message}); keeping the last reading of {ByteFormat.Format(last)}");
                fresh = false;
                return last;
            }
        }

        /// <summary>Grants the waiters that fit, taking the child processes' commit off the measurement only when the gauge alone holds one back.</summary>
        private void Evaluate(Measurement measured)
        {
            var outcome = GrantWaiters(measured.UsedBytes, startRecheck: measured.ChildrenAccountedFor);
            if (outcome.Blocked is not null && !measured.ChildrenAccountedFor)
            {
                measured = MeasureBeyondLeases();
                outcome = GrantWaiters(measured.UsedBytes, startRecheck: true);
            }

            if (outcome.Blocked is not { } blocked)
                return;

            bool reclaimed = TryReclaim();
            LogIfDue(blocked, reclaimed);
            if (outcome.ProgressPending)
                GrantForProgress(measured.UsedBytes, reclaimed);
        }

        /// <summary>A blocked outcome starts the periodic re-check only when <paramref name="startRecheck"/>, so a pass retried at once does not start it.</summary>
        private GrantOutcome GrantWaiters(long measuredBytes, bool startRecheck)
        {
            lock (_lock)
            {
                var outcome = GrantWaitersThatFit(measuredBytes, allowProgressGrant: false);
                if (startRecheck && outcome.Blocked is not null)
                    _recheckTimer ??= Clock.CreateTimer(_ => Reevaluate(), null, RecheckInterval, RecheckInterval);
                return outcome;
            }
        }

        /// <summary>No transient lease is held but the measurement still holds a waiter back: it is granted regardless, after a reclamation had its chance to make it fit.</summary>
        private void GrantForProgress(long measuredBytes, bool reclaimed)
        {
            if (reclaimed)
                measuredBytes = MeasureBeyondLeases().UsedBytes;

            lock (_lock)
            {
                GrantWaitersThatFit(measuredBytes, allowProgressGrant: true);
            }
        }

        /// <summary>Reclaims GC memory when the rate limit allows and the runtime holds enough unused memory for it to help.</summary>
        private bool TryReclaim()
        {
            if (!ReclaimIsDue() || ReclaimableBytes() < MinimumReclaimableBytes)
                return false;

            Reclaim();
            return true;
        }

        private bool ReclaimIsDue()
        {
            lock (_lock)
            {
                long now = Clock.GetTimestamp();
                if (_lastReclaimTimestamp is { } last && Clock.GetElapsedTime(last, now) < ReclaimInterval)
                    return false;
                _lastReclaimTimestamp = now;
                return true;
            }
        }

        private bool LogIsDue()
        {
            lock (_lock)
            {
                long now = Clock.GetTimestamp();
                if (_lastLogTimestamp is { } last && Clock.GetElapsedTime(last, now) < LogInterval)
                    return false;
                _lastLogTimestamp = now;
                return true;
            }
        }

        private void LogIfDue(BlockedEpisode blocked, bool reclaimed)
        {
            if (!LogIsDue())
                return;

            Log($"Memory: waiting for {ByteFormat.Format(blocked.RequestedBytes)}, {ByteFormat.Format(blocked.InUseBytes)} in use of "
                + $"{ByteFormat.Format(Capacity)} budget{(reclaimed ? "; reclaiming" : string.Empty)}");
        }

        /// <summary>
        /// Grants the waiters, oldest first, that fit beside the reserved and the measured bytes. A waiter that fits by reservation but
        /// not by measurement is reported as blocked; with no transient lease held it is granted anyway, one per pass, only when
        /// <paramref name="allowProgressGrant"/> is set, and otherwise reported as pending for that grant. An aged head that is not
        /// granted ends the scan, so nothing is granted ahead of it.
        /// </summary>
        private GrantOutcome GrantWaitersThatFit(long measuredBytes, bool allowProgressGrant)
        {
            BlockedEpisode? firstBlocked = null;
            bool progressPending = false;
            bool progressGranted = false;
            for (var node = _waiters.First; node is not null;)
            {
                var next = node.Next;
                var waiter = node.Value;
                long bytes = ClampToRoom(waiter.RequestedBytes);
                bool noTransientHeld = _reservedBytes == _longLivedBytes;
                bool fitsReservation = noTransientHeld || _reservedBytes + bytes <= Capacity;
                bool fitsMeasurement = Fits(bytes, measuredBytes);
                bool progressGrant = allowProgressGrant && noTransientHeld && !progressGranted;
                if (fitsReservation && (fitsMeasurement || progressGrant))
                {
                    _waiters.Remove(node);
                    progressGranted |= !fitsMeasurement;
                    waiter.Completion.SetResult(Take(bytes, waiter.Life));
                }
                else
                {
                    if (fitsReservation)
                    {
                        progressPending |= noTransientHeld;
                        firstBlocked ??= new BlockedEpisode(bytes, SaturateToLong(_reservedBytes + measuredBytes));
                    }

                    if (node == _waiters.First && IsAged(waiter))
                        break;
                }

                node = next;
            }

            if (firstBlocked is null)
            {
                EndBlockedEpisode();
                return default;
            }

            return new GrantOutcome(firstBlocked, progressPending);
        }

        private bool IsAged(Waiter waiter) => Clock.GetElapsedTime(waiter.EnqueuedTimestamp, Clock.GetTimestamp()) >= AgingThreshold;

        private bool Fits(long bytes, long measuredBytes) => _reservedBytes + measuredBytes + bytes <= Capacity;

        /// <summary>A request is clamped to what the long-lived leases leave of the capacity, and never below one byte.</summary>
        private long ClampToRoom(long requestedBytes) => (long)Int128.Max(1, Int128.Min(requestedBytes, Capacity - _longLivedBytes));

        private MemoryLease Take(long bytes, LeaseLife life)
        {
            _reservedBytes += bytes;
            if (life == LeaseLife.LongLived)
                _longLivedBytes += bytes;
            return new MemoryLease(this, bytes, life == LeaseLife.LongLived);
        }

        private void EndBlockedEpisode()
        {
            _recheckTimer?.Dispose();
            _recheckTimer = null;
        }

        private static MemoryLease[] Without(MemoryLease[] leases, MemoryLease removed)
        {
            int index = Array.IndexOf(leases, removed);
            return index < 0 ? leases : [.. leases[..index], .. leases[(index + 1)..]];
        }

        private static long SaturateToLong(Int128 bytes) => (long)Int128.Min(bytes, long.MaxValue);

        private static void ReclaimGcMemory() => GC.Collect(2, GCCollectionMode.Aggressive, blocking: true, compacting: true);

        /// <summary>The committed heap memory beyond the heap's size at the last collection, plus what was allocated since: what a compacting collection could return.</summary>
        private static long EstimateGcReclaimableBytes()
        {
            var info = GC.GetGCMemoryInfo();
            long unusedCommit = Math.Max(0, info.TotalCommittedBytes - info.HeapSizeBytes);
            long allocatedSinceCollection = Math.Max(0, GC.GetTotalMemory(forceFullCollection: false) - info.HeapSizeBytes);
            return unusedCommit + allocatedSinceCollection;
        }

        private enum LeaseLife : byte
        {
            Transient,
            LongLived,
        }

        private sealed record BlockedEpisode(long RequestedBytes, long InUseBytes);

        private readonly record struct GrantOutcome(BlockedEpisode? Blocked, bool ProgressPending);

        /// <param name="UsedBytes">The measured bytes.</param>
        /// <param name="ChildrenAccountedFor">Whether nothing is left to take off for the commit of the leases' child processes.</param>
        private readonly record struct Measurement(long UsedBytes, bool ChildrenAccountedFor);

        private sealed class Waiter
        {
            public Waiter(long requestedBytes, LeaseLife life)
            {
                RequestedBytes = requestedBytes;
                Life = life;
            }

            public long RequestedBytes { get; }

            public LeaseLife Life { get; }

            public long EnqueuedTimestamp { get; set; }

            public LinkedListNode<Waiter> Node { get; set; } = null!;

            public TaskCompletionSource<MemoryLease> Completion { get; } = new(TaskCreationOptions.RunContinuationsAsynchronously);
        }
    }
}
