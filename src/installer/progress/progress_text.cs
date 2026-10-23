namespace AssetConverter.Progress
{
    /// <summary>Display strings shared by the console and the window; the numbers and durations themselves come from <see cref="ProgressFormatter"/>.</summary>
    public static class ProgressText
    {
        public const string Skipped = "skipped";

        /// <summary>"3 of 9 steps done · 2 running" while running, an outcome sentence afterwards.</summary>
        public static string Overall(ProgressSnapshot snapshot)
        {
            int count = snapshot.Steps.Count;
            switch (snapshot.State)
            {
                case RunState.Succeeded:
                    return $"Installation complete — {count} of {count} steps";
                case RunState.Cancelled:
                    return $"Installation cancelled — {snapshot.DoneCount} of {count} steps completed";
                case RunState.Failed:
                    string failed = Names(snapshot, StepStatus.Failed);
                    return failed.Length > 0 ? $"Installation failed — {failed}" : "Installation failed";
                default:
                    int running = snapshot.Steps.Count(s => s.Status == StepStatus.Running);
                    string activity = running > 0 ? $"{running} running" : snapshot.DoneCount == 0 ? "Starting" : "Working";
                    return $"{snapshot.DoneCount} of {count} steps done · {activity}";
            }
        }

        private static string Names(ProgressSnapshot snapshot, StepStatus status) =>
            string.Join(", ", snapshot.Steps.Where(s => s.Status == status).Select(s => s.Name));
    }
}
