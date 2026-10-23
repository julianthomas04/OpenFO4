using System.Globalization;

namespace AssetConverter.Progress
{
    /// <summary>Formats the numbers and durations of progress snapshots with one culture; <see cref="Invariant"/> keeps console output stable.</summary>
    public sealed class ProgressFormatter(IFormatProvider provider)
    {
        public static ProgressFormatter Invariant { get; } = new(CultureInfo.InvariantCulture);

        public string Number(long value) => value.ToString("N0", provider);

        /// <summary>"1h 02m 03s", "4m 05s" or "6.7s"; the decimal separator of the short form follows the culture.</summary>
        public string Duration(TimeSpan t)
        {
            string separator = NumberFormatInfo.GetInstance(provider).NumberDecimalSeparator;
            return t.TotalHours >= 1 ? string.Create(provider, $"{(int)t.TotalHours}h {t.Minutes:D2}m {t.Seconds:D2}s") :
                   t.TotalMinutes >= 1 ? string.Create(provider, $"{(int)t.TotalMinutes}m {t.Seconds:D2}s") :
                                         string.Create(provider, $"{t.Seconds}{separator}{t.Milliseconds / 100}s");
        }

        /// <summary>"12,345 of 63,000 meshes"; without a known total "12,345 meshes".</summary>
        public string Detailed(StepSnapshot step)
        {
            string unit = step.Unit is { Length: > 0 } u ? " " + u : "";
            if (step.Total > 0) return $"{Number(step.Completed)} of {Number(step.Total)}{unit}";
            return step.Completed > 0 ? $"{Number(step.Completed)}{unit}" : "";
        }

        /// <summary>"12,345 / 63,000" for the step list; empty for a step that has not started, "skipped" for one that had no work.</summary>
        public string Compact(StepSnapshot step)
        {
            if (step.WasSkipped) return ProgressText.Skipped;
            if (step.Status == StepStatus.Pending) return "";
            if (step.Total > 0) return $"{Number(step.Completed)} / {Number(step.Total)}";
            return step.Completed > 0 ? Number(step.Completed) : "";
        }

        /// <summary>The time a step has been running or ran; empty for one that never started, also for one that only looked for work.</summary>
        public string Elapsed(StepSnapshot step) =>
            step.StartedAt == null ? "" : Duration(step.Elapsed);

        /// <summary>The share of <paramref name="fraction"/>, clamped to 0 to 1 and rounded down: "37%".</summary>
        public string Percent(double fraction) =>
            Number((long)Math.Floor(Math.Clamp(fraction, 0.0, 1.0) * 100)) + NumberFormatInfo.GetInstance(provider).PercentSymbol;
    }
}
