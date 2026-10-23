using AssetConverter.Progress;

namespace AssetConverter.Pipeline
{
    /// <summary>
    /// Starts the concurrent lanes of the install. Every lane goes through <see cref="InstallProgress.RunLane"/>, waits for exactly the
    /// tasks and gates it was given, checks for cancellation once they are ready, and records its phase in the <see cref="PhaseLog"/>.
    /// A lane that ends without opening a gate it is responsible for fails, and every such gate is faulted with the exception that ended
    /// the lane, so the lanes behind it fail with that same exception instead of waiting for ever.
    /// </summary>
    internal sealed class LaneRunner
    {
        private readonly PhaseLog _phases;
        private readonly CancellationToken _ct;

        public LaneRunner(PhaseLog phases, CancellationToken ct)
        {
            _phases = phases;
            _ct = ct;
        }

        /// <param name="steps">The progress steps the lane owns; a failing lane marks its running ones failed.</param>
        /// <param name="work">The lane's work, started once everything in <paramref name="after"/> succeeded.</param>
        /// <param name="phase">Name under which the work is recorded in the phase log; a lane that records its own phases passes none.</param>
        /// <param name="after">The gates and lanes whose output the work reads.</param>
        /// <param name="opens">The gates the work opens, faulted when the lane ends without opening them.</param>
        public Task Run(IReadOnlyList<string> steps, Action work, string? phase = null,
            IReadOnlyList<Task>? after = null, IReadOnlyList<Gate>? opens = null) =>
            Run(steps, () =>
            {
                work();
                return Task.CompletedTask;
            }, phase, after, opens);

        public Task Run(IReadOnlyList<string> steps, Func<Task> work, string? phase = null,
            IReadOnlyList<Task>? after = null, IReadOnlyList<Gate>? opens = null) =>
            Execute(steps, async () =>
            {
                await work().ConfigureAwait(false);
                return 0;
            }, phase, after, opens);

        public Task<T> Run<T>(IReadOnlyList<string> steps, Func<T> work, string? phase = null,
            IReadOnlyList<Task>? after = null, IReadOnlyList<Gate>? opens = null) =>
            Execute(steps, () => Task.FromResult(work()), phase, after, opens);

        private async Task<T> Execute<T>(IReadOnlyList<string> steps, Func<Task<T>> work, string? phase,
            IReadOnlyList<Task>? after, IReadOnlyList<Gate>? opens)
        {
            after ??= [];
            opens ??= [];
            T result = default!;
            try
            {
                await InstallProgress.RunLane(steps, () => Task.Run(async () =>
                {
                    await Task.WhenAll(after).ConfigureAwait(false);
                    _ct.ThrowIfCancellationRequested();
                    var start = _phases.Elapsed;
                    result = await work().ConfigureAwait(false);
                    RequireOpened(opens);
                    if (phase != null) _phases.Record(phase, start);
                })).ConfigureAwait(false);
            }
            catch (Exception ex)
            {
                foreach (var gate in opens) gate.Abort(ex);
                throw;
            }
            return result;
        }

        private static void RequireOpened(IReadOnlyList<Gate> gates)
        {
            var unopened = gates.FirstOrDefault(gate => !gate.IsOpen);
            if (unopened != null)
                throw new InvalidOperationException($"A lane finished without opening the '{unopened.Name}' gate that other lanes wait for.");
        }
    }
}
