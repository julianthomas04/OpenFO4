namespace AssetConverter.Progress
{
    public enum StepStatus
    {
        Pending,
        Running,
        Done,
        Failed,
        Cancelled,
    }

    public enum RunState
    {
        Running,
        Succeeded,
        Failed,
        Cancelled,
    }

    /// <summary>Immutable view of one step at the moment a snapshot was taken. Times are offsets from the start of the run.</summary>
    public sealed record StepSnapshot(
        string Name,
        string? Unit,
        StepStatus Status,
        long Completed,
        long Total,
        string? Detail,
        TimeSpan? StartedAt,
        TimeSpan? FinishedAt,
        TimeSpan Elapsed,
        double Fraction)
    {
        /// <summary>The units of work a grouped step is made of, in declaration order; empty for a step that is a single unit.</summary>
        public IReadOnlyList<StepSnapshot> Parts { get; init; } = [];

        /// <summary>The step is running but still discovering its work: its total is not final yet, so it shows no counters and no fill.</summary>
        public bool IsPreparing { get; init; }

        /// <summary>Whether <see cref="Total"/> can no longer change: for a step that has not begun, only when its total was declared up front.</summary>
        public required bool TotalIsFinal { get; init; }

        /// <summary>A step that finished without ever having work: it never started, or its phases ended with no total and nothing completed.</summary>
        public bool WasSkipped => Status == StepStatus.Done && (StartedAt == null || (Total <= 0 && Completed <= 0));
    }

    /// <summary>Immutable view of the whole run.</summary>
    public sealed record ProgressSnapshot(
        IReadOnlyList<StepSnapshot> Steps,
        double OverallFraction,
        TimeSpan Elapsed,
        RunState State)
    {
        public int DoneCount => Steps.Count(s => s.Status == StepStatus.Done);

        public bool IsFinished => State != RunState.Running;
    }
}
