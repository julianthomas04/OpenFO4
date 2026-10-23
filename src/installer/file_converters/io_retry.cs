namespace AssetConverter
{
    /// <summary>Retries a file operation a file scanner or indexer made fail for a moment, pausing a little longer after each failure.</summary>
    internal static class IoRetry
    {
        public static IReadOnlyList<TimeSpan> DefaultDelays { get; } =
            [TimeSpan.FromMilliseconds(50), TimeSpan.FromMilliseconds(100), TimeSpan.FromMilliseconds(200), TimeSpan.FromMilliseconds(400)];

        private const int Win32ErrorFacility = unchecked((int)0x80070000);
        private const int Win32ErrorMask = 0xFFFF;
        private const int ErrorSharingViolation = 32;
        private const int ErrorLockViolation = 33;

        /// <summary>A file another process holds open or locked (a sharing or lock violation), as opposed to a failure waiting will not cure, like a full disk or a path too long.</summary>
        public static bool IsFileBusy(Exception ex) =>
            ex is IOException && (ex.HResult & ~Win32ErrorMask) == Win32ErrorFacility
            && (ex.HResult & Win32ErrorMask) is ErrorSharingViolation or ErrorLockViolation;

        public static bool IsFileBusyOrDenied(Exception ex) => IsFileBusy(ex) || ex is UnauthorizedAccessException;

        /// <summary>
        /// Runs <paramref name="operation"/>; when it throws what <paramref name="isTransient"/> accepts it is run again after each of
        /// <paramref name="delays"/>, and the exception of the last attempt is rethrown.
        /// </summary>
        /// <param name="wait">Pauses between attempts; defaults to sleeping, replaceable so tests need not.</param>
        public static T Run<T>(Func<T> operation, Func<Exception, bool> isTransient, IReadOnlyList<TimeSpan> delays,
            Action<TimeSpan>? wait = null, CancellationToken ct = default)
        {
            wait ??= Thread.Sleep;
            for (int attempt = 0; ; attempt++)
            {
                try
                {
                    return operation();
                }
                catch (Exception ex) when (attempt < delays.Count && isTransient(ex))
                {
                }
                wait(delays[attempt]);
                ct.ThrowIfCancellationRequested();
            }
        }

        public static void Run(Action operation, Func<Exception, bool> isTransient, IReadOnlyList<TimeSpan> delays,
            Action<TimeSpan>? wait = null, CancellationToken ct = default) =>
            Run(() =>
            {
                operation();
                return 0;
            }, isTransient, delays, wait, ct);
    }
}
