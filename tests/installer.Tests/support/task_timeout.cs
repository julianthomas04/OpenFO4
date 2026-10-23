using Xunit;

namespace InstallerTests
{
    internal static class TaskTimeout
    {
        private static readonly TimeSpan Limit = TimeSpan.FromSeconds(10);

        /// <summary>Fails a test that waits for a task that never completes instead of hanging the run; also stops waiting when the test run is cancelled.</summary>
        public static Task WithTimeout(this Task task) => task.WaitAsync(Limit, TestContext.Current.CancellationToken);

        public static Task<T> WithTimeout<T>(this Task<T> task) => task.WaitAsync(Limit, TestContext.Current.CancellationToken);
    }
}
