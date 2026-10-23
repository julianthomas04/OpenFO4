namespace AssetConverter.Logging
{
    /// <summary>Makes sure a task nobody awaits cannot fail unnoticed: its exception goes to the error log as soon as it happens.</summary>
    public static class TaskFaultReporter
    {
        /// <summary>Reports the exception of <paramref name="task"/>, if it faults, to <see cref="Console.Error"/> (and so to the log). A cancellation is not a fault.</summary>
        public static void Observe(Task task, string description)
        {
            ArgumentNullException.ThrowIfNull(task);
            task.ContinueWith(
                failed => Console.Error.WriteLine($"ERROR: {description} failed: {failed.Exception!.Flatten()}"),
                CancellationToken.None,
                TaskContinuationOptions.OnlyOnFaulted | TaskContinuationOptions.ExecuteSynchronously,
                TaskScheduler.Default);
        }
    }
}
