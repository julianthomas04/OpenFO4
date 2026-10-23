namespace AssetConverter.Progress
{
    /// <summary>
    /// Folds the units of one group into the single step shown for it. The step is running while any unit runs, or while some units are
    /// done and others have not begun (the group has started but is not complete); it is done only when every unit is done, and a
    /// failed or cancelled unit fails or cancels it. Counters add up and the fraction is the weight-averaged fraction of the units, so
    /// a unit that has not begun yet keeps its share of the bar.
    /// </summary>
    internal static class StepGrouping
    {
        private const string MixedUnit = "items";

        public static StepSnapshot Fold(string name, IReadOnlyList<StepSnapshot> parts, IReadOnlyList<double> weights, TimeSpan now)
        {
            var status = StatusOf(parts);
            TimeSpan? startedAt = parts.Where(p => p.StartedAt != null).Select(p => p.StartedAt).Min();
            bool finished = parts.All(p => p.Status is StepStatus.Done or StepStatus.Failed or StepStatus.Cancelled);
            TimeSpan? finishedAt = finished && parts.All(p => p.FinishedAt != null) ? parts.Max(p => p.FinishedAt) : null;
            TimeSpan elapsed = startedAt is { } start ? (finishedAt ?? now) - start : TimeSpan.Zero;
            var units = parts.Select(p => p.Unit).Where(u => u != null).Distinct().ToList();

            return new StepSnapshot(
                name,
                units.Count == 1 ? units[0] : MixedUnit,
                status,
                parts.Sum(p => p.Completed),
                parts.Sum(p => p.Total),
                parts.FirstOrDefault(p => p.Status == StepStatus.Running && p.Detail != null)?.Detail,
                startedAt,
                finishedAt,
                elapsed,
                FractionOf(status, parts, weights))
            {
                Parts = parts,
            };
        }

        private static StepStatus StatusOf(IReadOnlyList<StepSnapshot> parts)
        {
            if (parts.Any(p => p.Status == StepStatus.Failed)) return StepStatus.Failed;
            if (parts.Any(p => p.Status == StepStatus.Cancelled)) return StepStatus.Cancelled;
            if (parts.All(p => p.Status == StepStatus.Done)) return StepStatus.Done;
            return parts.All(p => p.Status == StepStatus.Pending) ? StepStatus.Pending : StepStatus.Running;
        }

        private static double FractionOf(StepStatus status, IReadOnlyList<StepSnapshot> parts, IReadOnlyList<double> weights)
        {
            if (status == StepStatus.Done) return 1.0;
            double weighted = 0, total = 0;
            for (int i = 0; i < parts.Count; i++)
            {
                weighted += weights[i] * parts[i].Fraction;
                total += weights[i];
            }
            double fraction = total > 0 ? weighted / total : 0.0;
            return status == StepStatus.Running ? Math.Min(ProgressModel.RunningFractionCap, fraction) : fraction;
        }
    }
}
