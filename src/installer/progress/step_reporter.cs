namespace AssetConverter.Progress
{
    /// <summary>Writes one line when a step starts and one when it ends, with the run offset and the step's duration and counters; a step that had no work gets a single "Skipped" line unless it was already announced as started.</summary>
    public sealed class StepReporter : IAsyncDisposable
    {
        private enum Reported
        {
            Nothing,
            Started,
            Ended,
        }

        private static readonly TimeSpan PollInterval = TimeSpan.FromMilliseconds(250);

        private readonly ProgressModel _model;
        private readonly TextWriter _output;
        private readonly Action<string> _onFault;
        private readonly List<Reported> _reported = new();
        private readonly CancellationTokenSource _stop = new();
        private Task? _loop;
        private bool _disposed;
        private bool _faulted;

        /// <param name="onFault">Receives the one message when the reporter stops because writing a line failed; defaults to <see cref="Console.Error"/>.</param>
        public StepReporter(ProgressModel model, TextWriter output, Action<string>? onFault = null)
        {
            ArgumentNullException.ThrowIfNull(model);
            ArgumentNullException.ThrowIfNull(output);
            _model = model;
            _output = output;
            _onFault = onFault ?? Console.Error.WriteLine;
        }

        public void Start()
        {
            _loop = Task.Run(PollUntilStopped);
        }

        public async ValueTask DisposeAsync()
        {
            if (_disposed) return;
            _disposed = true;
            await _stop.CancelAsync().ConfigureAwait(false);
            if (_loop != null) await _loop.ConfigureAwait(false);
            _stop.Dispose();
            TryPoll();
        }

        public void Poll()
        {
            var steps = _model.GetSnapshot().Steps;
            while (_reported.Count < steps.Count) _reported.Add(Reported.Nothing);
            for (int i = 0; i < steps.Count; i++)
            {
                var step = steps[i];
                if (_reported[i] == Reported.Ended) continue;
                if (_reported[i] == Reported.Nothing && step.StartedAt != null && !step.WasSkipped)
                {
                    _output.WriteLine($"[{Format(step.StartedAt.Value)}] Started: {step.Name}");
                    _reported[i] = Reported.Started;
                }
                if (step.Status is StepStatus.Done or StepStatus.Failed or StepStatus.Cancelled)
                {
                    _output.WriteLine(EndLine(step, announced: _reported[i] == Reported.Started));
                    _reported[i] = Reported.Ended;
                }
            }
        }

        private static string Format(TimeSpan time) => ProgressFormatter.Invariant.Duration(time);

        private static string EndLine(StepSnapshot step, bool announced)
        {
            string at = Format(step.FinishedAt ?? TimeSpan.Zero);
            if (step.StartedAt == null || (step.WasSkipped && !announced)) return $"[{at}] {(step.WasSkipped ? "Skipped" : "Not started")}: {step.Name}";
            string duration = Format(step.Elapsed);
            string counters = ProgressFormatter.Invariant.Detailed(step);
            string suffix = counters.Length > 0 ? $" ({counters})" : "";
            return step.Status switch
            {
                StepStatus.Done => $"[{at}] Finished: {step.Name} in {duration}{suffix}",
                StepStatus.Failed => $"[{at}] FAILED: {step.Name} after {duration}{suffix}",
                _ => $"[{at}] Cancelled: {step.Name} after {duration}{suffix}",
            };
        }

        private async Task PollUntilStopped()
        {
            using var timer = new PeriodicTimer(PollInterval);
            try
            {
                while (await timer.WaitForNextTickAsync(_stop.Token).ConfigureAwait(false)) TryPoll();
            }
            catch (OperationCanceledException)
            {
                // Stopping is the normal way for the loop to end.
            }
        }

        /// <summary>
        /// The step lines are informational: whatever makes writing them fail (closed console, full disk, a defect) is reported once
        /// and ends the lines for the rest of the run instead of the run itself.
        /// </summary>
        private void TryPoll()
        {
            if (_faulted) return;
            try
            {
                Poll();
            }
            catch (Exception ex)
            {
                _faulted = true;
                _onFault($"WARNING: the step progress lines stopped; the installer keeps running. {ex}");
            }
        }
    }
}
