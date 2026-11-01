using System.Globalization;

namespace AssetConverter.Progress
{
    /// <summary>
    /// Thread-safe, UI-independent state of one installation: declared steps with counters, status and timing. Writers
    /// (<see cref="BeginPhase"/>, <see cref="Tick"/>, ...) may be called from any thread; readers poll <see cref="GetSnapshot"/>.
    /// A phase is one unit of reported work bound to the declared step of the same name; a step finishes when its open phases end.
    /// </summary>
    public sealed class ProgressModel
    {
        /// <summary>A running step never reads as complete: it reads 100% only once its phases have ended, so a step whose last tick landed but whose work is still wrapping up stays just short.</summary>
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
        /// A number for <paramref name="total"/> is the phase's final total, a <c>null</c> total means the phase is still discovering its
        /// work: it reports as preparing until <see cref="SealTotal"/> (or <see cref="EndPhase"/>) fixes the total, and work can only
        /// be added with <see cref="AddTotal"/> while it is open. A step announced with <see cref="Declare"/> must begin with exactly that total.
        /// Returns the phase id, or -1 once the run has finished.
        /// </summary>
        public int BeginPhase(string name, long? total, string? unit)
        {
            ArgumentException.ThrowIfNullOrWhiteSpace(name);
            if (total < 0) throw new ArgumentOutOfRangeException(nameof(total), total, "A total cannot be negative.");
            lock (_gate)
            {
                if (_state != RunState.Running) return -1;
                var step = FindStep(name) ?? AddStep(new StepDefinition(name, unit));
                if (step.DeclaredTotal is { } declared)
                {
                    if (total != declared)
                        throw new InvalidOperationException($"Step '{step.Name}' was declared with a total of {declared} but began with {(total.HasValue ? total.Value.ToString(CultureInfo.InvariantCulture) : "no total")}.");
                    step.DeclaredTotal = null;
                }
                else if (total is > 0)
                {
                    Interlocked.Add(ref step.Total, total.Value);
                }
                step.Unit ??= unit;
                int id = _phases.Length;
                _phases = [.. _phases, new PhaseState(step) { Sealed = total != null }];
                step.OpenPhases++;
                if (total == null) step.UnsealedPhases++;
                if (step.Status is StepStatus.Pending or StepStatus.Done)
                {
                    step.Status = StepStatus.Running;
                    step.StartTimestamp ??= _clock.GetTimestamp();
                    step.FinishTimestamp = null;
                    step.Final = null;
                }
                return id;
            }
        }

        /// <summary>
        /// Announces the total of a step that has not begun, so a group it belongs to can tell its total is known before the step runs.
        /// The step's first phase must begin with the same total.
        /// </summary>
        public void Declare(string stepName, long total)
        {
            ArgumentException.ThrowIfNullOrWhiteSpace(stepName);
            ArgumentOutOfRangeException.ThrowIfNegative(total);
            lock (_gate)
            {
                var step = FindStep(stepName) ?? throw new ArgumentException($"Step '{stepName}' is not declared.", nameof(stepName));
                if (step.Status != StepStatus.Pending)
                    throw new InvalidOperationException($"Step '{step.Name}' has already begun, its total cannot be declared.");
                if (step.DeclaredTotal is { } known && known != total)
                    throw new InvalidOperationException($"Step '{step.Name}' is already declared with a total of {known}, not {total}.");
                step.DeclaredTotal = total;
                Interlocked.Exchange(ref step.Total, total);
            }
        }

        /// <summary>Adds work to a phase that is still discovering its total. Throws once the total is sealed or the phase ended.</summary>
        public void AddTotal(int phase, long count)
        {
            lock (_gate)
            {
                if (TryGetPhase(phase) is not { } state) return;
                if (state.Sealed)
                    throw new InvalidOperationException($"The total of step '{state.Step.Name}' is sealed, work can no longer be added.");
                Interlocked.Add(ref state.Step.Total, count);
            }
        }

        /// <summary>Fixes the total of a discovering phase; the step stops preparing and starts to fill. Sealing twice is harmless.</summary>
        public void SealTotal(int phase)
        {
            lock (_gate)
            {
                if (TryGetPhase(phase) is { } state) Seal(state);
            }
        }

        /// <summary>Records progress on a phase. Throws while the phase is still discovering its total, since the fraction would be meaningless.</summary>
        public void Tick(int phase, long count = 1)
        {
            if (TryGetPhase(phase) is not { } state) return;
            if (!state.Sealed)
                throw new InvalidOperationException($"Step '{state.Step.Name}' ticked before its total was sealed.");
            Interlocked.Add(ref state.Step.Completed, count);
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
                Seal(phases[phase]);
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

        private static void Seal(PhaseState state)
        {
            if (state.Sealed) return;
            state.Sealed = true;
            state.Step.UnsealedPhases--;
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
            bool totalIsFinal = TotalIsFinal(step.Status, step.UnsealedPhases, step.DeclaredTotal);
            bool preparing = step.Status == StepStatus.Running && !totalIsFinal;
            return new StepSnapshot(step.Name, step.Unit, step.Status, completed, total, step.Detail,
                startedAt, finishedAt, elapsed, StepFraction(step.Status, preparing, completed, total))
            {
                IsPreparing = preparing,
                TotalIsFinal = totalIsFinal,
            };
        }

        /// <summary>Whether a step's total can still change: a pending step knows it only when declared, a running one once none of its phases is discovering, a finished one always.</summary>
        internal static bool TotalIsFinal(StepStatus status, int unsealedPhases, long? declaredTotal) => status switch
        {
            StepStatus.Pending => declaredTotal != null,
            StepStatus.Running => unsealedPhases == 0,
            _ => true,
        };

        internal static double StepFraction(StepStatus status, bool preparing, long completed, long total)
        {
            if (status == StepStatus.Done) return 1.0;
            if (preparing || total <= 0 || completed <= 0) return 0.0;
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

        private StepState? TryGetStep(int phase) => TryGetPhase(phase)?.Step;

        private PhaseState? TryGetPhase(int phase)
        {
            var phases = _phases;
            return (uint)phase < (uint)phases.Length ? phases[phase] : null;
        }

        private StepState? FindStep(string name) => _steps.Find(s => string.Equals(s.Name, name, StringComparison.OrdinalIgnoreCase));

        private StepState AddStep(StepDefinition definition)
        {
            var step = NewStep(definition);
            _steps.Add(step);
            return step;
        }

        private static StepState NewStep(StepDefinition definition) =>
            new(definition.Name, definition.Unit, definition.Weight, definition.VisibleName)
            {
                DeclaredTotal = definition.KnownTotal,
                Total = definition.KnownTotal ?? 0,
            };

        private sealed class StepState(string name, string? unit, double weight, string visibleName)
        {
            public readonly string Name = name;
            public readonly string VisibleName = visibleName;
            public readonly double Weight = weight;
            public string? Unit = unit;
            public StepStatus Status;
            public long Total;
            public long Completed;
            public long? DeclaredTotal;
            public int UnsealedPhases;
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
            public volatile bool Sealed;
        }
    }
}
