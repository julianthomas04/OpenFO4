namespace AssetConverter.Progress
{
    /// <summary>
    /// Thread-safe, UI-independent state of one installation: declared steps with counters, status and timing. Writers
    /// (<see cref="BeginPhase"/>, <see cref="Tick"/>, ...) may be called from any thread; readers poll <see cref="GetSnapshot"/>.
    /// A phase is one unit of reported work bound to the declared step of the same name; a step finishes when its open phases end.
    /// </summary>
    public sealed class ProgressModel
    {
        /// <summary>A running step never reads as complete: its total may still grow, and only ending the step completes it.</summary>
        public const double RunningFractionCap = 0.99;

        private readonly object _gate = new();
        private readonly TimeProvider _clock;
        private readonly long _startTimestamp;
        private readonly List<StepState> _steps = new();
        private volatile PhaseState[] _phases = [];
        private long? _finishTimestamp;
        private RunState _state = RunState.Running;
        private double _overallHighWater;

        public ProgressModel(IReadOnlyList<StepDefinition> steps, TimeProvider? clock = null)
        {
            ArgumentNullException.ThrowIfNull(steps);
            _clock = clock ?? TimeProvider.System;
            _startTimestamp = _clock.GetTimestamp();
            foreach (var definition in steps)
            {
                if (!(definition.Weight > 0) || double.IsInfinity(definition.Weight))
                    throw new ArgumentException($"Step '{definition.Name}' needs a positive finite weight, got {definition.Weight}.", nameof(steps));
                if (FindStep(definition.Name) != null)
                    throw new ArgumentException($"Step '{definition.Name}' is declared twice.", nameof(steps));
                _steps.Add(NewStep(definition));
            }
        }

        public RunState State
        {
            get { lock (_gate) return _state; }
        }

        /// <summary>
        /// Starts a unit of work for the declared step called <paramref name="name"/>; an undeclared name appends a new step.
        /// Returns the phase id, or -1 once the run has finished.
        /// </summary>
        public int BeginPhase(string name, long total, string? unit)
        {
            ArgumentException.ThrowIfNullOrWhiteSpace(name);
            lock (_gate)
            {
                if (_state != RunState.Running) return -1;
                var step = FindStep(name) ?? AddStep(new StepDefinition(name, unit));
                step.Unit ??= unit;
                int id = _phases.Length;
                _phases = [.. _phases, new PhaseState(step)];
                step.OpenPhases++;
                if (step.Status is StepStatus.Pending or StepStatus.Done)
                {
                    step.Status = StepStatus.Running;
                    step.StartTimestamp ??= _clock.GetTimestamp();
                    step.FinishTimestamp = null;
                    step.Final = null;
                }
                if (total > 0) Interlocked.Add(ref step.Total, total);
                return id;
            }
        }

        public void AddTotal(int phase, long count)
        {
            if (TryGetStep(phase) is { } step) Interlocked.Add(ref step.Total, count);
        }

        public void Tick(int phase, long count = 1)
        {
            if (TryGetStep(phase) is { } step) Interlocked.Add(ref step.Completed, count);
        }

        public void SetDetail(int phase, string? text)
        {
            if (TryGetStep(phase) is { } step) step.Detail = text;
        }

        public void EndPhase(int phase)
        {
            lock (_gate)
            {
                var phases = _phases;
                if ((uint)phase >= (uint)phases.Length || phases[phase].Ended) return;
                phases[phase].Ended = true;
                var step = phases[phase].Step;
                if (--step.OpenPhases == 0 && step.Status == StepStatus.Running)
                    Terminate(step, StepStatus.Done);
            }
        }

        /// <summary>Marks the running steps among <paramref name="stepNames"/> as failed.</summary>
        public void Fail(IEnumerable<string> stepNames) => TerminateRunning(stepNames, StepStatus.Failed);

        /// <summary>Marks the running steps among <paramref name="stepNames"/> as cancelled.</summary>
        public void Cancel(IEnumerable<string> stepNames) => TerminateRunning(stepNames, StepStatus.Cancelled);

        /// <summary>
        /// Ends the run. After success every step is done (a step that never reported work had nothing to do); otherwise steps
        /// still running or pending are cancelled.
        /// </summary>
        public void Finish(RunState outcome)
        {
            if (outcome == RunState.Running) throw new ArgumentException("A run cannot finish as Running.", nameof(outcome));
            lock (_gate)
            {
                if (_state != RunState.Running) return;
                long now = _clock.GetTimestamp();
                foreach (var step in _steps)
                {
                    if (step.Status is not (StepStatus.Pending or StepStatus.Running)) continue;
                    Terminate(step, outcome == RunState.Succeeded ? StepStatus.Done : StepStatus.Cancelled, now);
                }
                _finishTimestamp = now;
                _state = outcome;
            }
        }

        public ProgressSnapshot GetSnapshot()
        {
            lock (_gate)
            {
                long now = _finishTimestamp ?? _clock.GetTimestamp();
                var units = new StepSnapshot[_steps.Count];
                for (int i = 0; i < units.Length; i++) units[i] = Snapshot(_steps[i], now);

                _overallHighWater = Math.Max(_overallHighWater, OverallFraction(units));
                double overall = _state == RunState.Succeeded ? 1.0 : _overallHighWater;
                return new ProgressSnapshot(Visible(units, Offset(now)), overall, Offset(now), _state);
            }
        }

        /// <summary>The steps as shown: units sharing a group are folded into one step at the position of the group's first unit.</summary>
        private List<StepSnapshot> Visible(StepSnapshot[] units, TimeSpan now)
        {
            var visible = new List<StepSnapshot>();
            foreach (var group in Enumerable.Range(0, units.Length).GroupBy(i => _steps[i].VisibleName, StringComparer.OrdinalIgnoreCase))
            {
                var members = group.ToList();
                visible.Add(members.Count == 1 && units[members[0]].Name == group.Key
                    ? units[members[0]]
                    : StepGrouping.Fold(group.Key, members.Select(i => units[i]).ToList(), members.Select(i => _steps[i].Weight).ToList(), now));
            }
            return visible;
        }

        private StepSnapshot Snapshot(StepState step, long now)
        {
            long completed = step.Final?.Completed ?? Interlocked.Read(ref step.Completed);
            long total = step.Final?.Total ?? Interlocked.Read(ref step.Total);
            TimeSpan? startedAt = step.StartTimestamp is { } start ? Offset(start) : null;
            TimeSpan? finishedAt = step.FinishTimestamp is { } finish ? Offset(finish) : null;
            TimeSpan elapsed = step.StartTimestamp is { } begin
                ? _clock.GetElapsedTime(begin, step.FinishTimestamp ?? now)
                : TimeSpan.Zero;
            return new StepSnapshot(step.Name, step.Unit, step.Status, completed, total, step.Detail,
                startedAt, finishedAt, elapsed, StepFraction(step.Status, completed, total));
        }

        private static double StepFraction(StepStatus status, long completed, long total)
        {
            if (status == StepStatus.Done) return 1.0;
            if (total <= 0 || completed <= 0) return 0.0;
            double fraction = (double)completed / total;
            return status == StepStatus.Running ? Math.Min(RunningFractionCap, fraction) : Math.Min(1.0, fraction);
        }

        private double OverallFraction(StepSnapshot[] steps)
        {
            double weighted = 0, weights = 0;
            for (int i = 0; i < steps.Length; i++)
            {
                double weight = _steps[i].Weight;
                weighted += weight * steps[i].Fraction;
                weights += weight;
            }
            return weights > 0 ? Math.Clamp(weighted / weights, 0.0, 1.0) : 0.0;
        }

        private void TerminateRunning(IEnumerable<string> stepNames, StepStatus status)
        {
            lock (_gate)
            {
                foreach (string name in stepNames)
                {
                    if (FindStep(name) is { Status: StepStatus.Running } step) Terminate(step, status);
                }
            }
        }

        private void Terminate(StepState step, StepStatus status, long? at = null)
        {
            step.Status = status;
            step.FinishTimestamp = at ?? _clock.GetTimestamp();
            step.Final = new Counters(Interlocked.Read(ref step.Completed), Interlocked.Read(ref step.Total));
        }

        private TimeSpan Offset(long timestamp) => _clock.GetElapsedTime(_startTimestamp, timestamp);

        private StepState? TryGetStep(int phase)
        {
            var phases = _phases;
            return (uint)phase < (uint)phases.Length ? phases[phase].Step : null;
        }

        private StepState? FindStep(string name) => _steps.Find(s => string.Equals(s.Name, name, StringComparison.OrdinalIgnoreCase));

        private StepState AddStep(StepDefinition definition)
        {
            var step = NewStep(definition);
            _steps.Add(step);
            return step;
        }

        private static StepState NewStep(StepDefinition definition) =>
            new(definition.Name, definition.Unit, definition.Weight, definition.VisibleName);

        private sealed class StepState(string name, string? unit, double weight, string visibleName)
        {
            public readonly string Name = name;
            public readonly string VisibleName = visibleName;
            public readonly double Weight = weight;
            public string? Unit = unit;
            public StepStatus Status;
            public long Total;
            public long Completed;
            public volatile string? Detail;
            public int OpenPhases;
            public long? StartTimestamp;
            public long? FinishTimestamp;
            public Counters? Final;
        }

        private sealed record Counters(long Completed, long Total);

        private sealed class PhaseState(StepState step)
        {
            public readonly StepState Step = step;
            public bool Ended;
        }
    }
}
