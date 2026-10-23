using AssetConverter;
using Xunit;

namespace InstallerTests
{
    public class IoRetryTests
    {
        private static readonly TimeSpan[] Delays = [TimeSpan.FromMilliseconds(1), TimeSpan.FromMilliseconds(2), TimeSpan.FromMilliseconds(3)];

        private static CancellationToken Ct => TestContext.Current.CancellationToken;

        private const int ErrorSharingViolation = unchecked((int)0x80070020);
        private const int ErrorLockViolation = unchecked((int)0x80070021);
        private const int ErrorDiskFull = unchecked((int)0x80070070);

        private static IOException Busy(string message = "held") => new(message, ErrorSharingViolation);

        [Fact]
        public void AnOperationThatSucceedsAtOnceIsNotRetried()
        {
            var waits = new List<TimeSpan>();

            int result = IoRetry.Run(() => 7, IoRetry.IsFileBusy, Delays, waits.Add, Ct);

            Assert.Equal(7, result);
            Assert.Empty(waits);
        }

        [Fact]
        public void AFailureThatEndsIsRetriedAfterEachDelayInTurnAndTheResultReturned()
        {
            int attempts = 0;
            var waits = new List<TimeSpan>();

            int result = IoRetry.Run(() => ++attempts < 3 ? throw Busy() : attempts, IoRetry.IsFileBusy, Delays, waits.Add, Ct);

            Assert.Equal(3, result);
            Assert.Equal(Delays[..2], waits);
        }

        [Fact]
        public void AFailureThatNeverEndsIsRethrownAfterTheLastDelay()
        {
            int attempts = 0;
            var waits = new List<TimeSpan>();

            var thrown = Assert.Throws<IOException>(() => IoRetry.Run<int>(() => throw Busy($"attempt {++attempts}"), IoRetry.IsFileBusy, Delays, waits.Add, Ct));

            Assert.Equal("attempt 4", thrown.Message);
            Assert.Equal(Delays, waits);
        }

        [Fact]
        public void AFileThatIsNotThereIsNotWaitedFor()
        {
            var waits = new List<TimeSpan>();

            Assert.Throws<FileNotFoundException>(() => IoRetry.Run<int>(() => throw new FileNotFoundException(), IoRetry.IsFileBusy, Delays, waits.Add, Ct));

            Assert.Empty(waits);
        }

        [Fact]
        public void OnlyASharingOrLockViolationIsABusyFile()
        {
            Assert.True(IoRetry.IsFileBusy(new IOException("held", ErrorSharingViolation)));
            Assert.True(IoRetry.IsFileBusy(new IOException("locked", ErrorLockViolation)));
            Assert.False(IoRetry.IsFileBusy(new IOException("full", ErrorDiskFull)));
            Assert.False(IoRetry.IsFileBusy(new IOException("unspecified")));
            Assert.False(IoRetry.IsFileBusy(new PathTooLongException()));
            Assert.False(IoRetry.IsFileBusy(new FileNotFoundException()));
            Assert.False(IoRetry.IsFileBusy(new DirectoryNotFoundException()));
            Assert.False(IoRetry.IsFileBusy(new UnauthorizedAccessException()));
            Assert.False(IoRetry.IsFileBusy(new InvalidOperationException("held", new IOException("held", ErrorSharingViolation))));
        }

        [Fact]
        public void AccessDeniedCountsAsBusyOnlyWhereTheCallerAcceptsIt()
        {
            Assert.True(IoRetry.IsFileBusyOrDenied(new UnauthorizedAccessException()));
            Assert.True(IoRetry.IsFileBusyOrDenied(Busy()));
            Assert.False(IoRetry.IsFileBusyOrDenied(new IOException("full", ErrorDiskFull)));
        }

        [Fact]
        public void ADiskFullFailureIsNotWaitedFor()
        {
            var waits = new List<TimeSpan>();

            Assert.Throws<IOException>(() => IoRetry.Run<int>(() => throw new IOException("full", ErrorDiskFull), IoRetry.IsFileBusy, Delays, waits.Add, Ct));

            Assert.Empty(waits);
        }

        [Fact]
        public void AFileHeldByAnotherWriterCanBeOpenedOnceItIsReleased()
        {
            string path = Path.Combine(Path.GetTempPath(), $"io_retry_{Guid.NewGuid():N}.bin");
            File.WriteAllBytes(path, [1, 2, 3]);
            try
            {
                var holder = new FileStream(path, FileMode.Open, FileAccess.ReadWrite, FileShare.None);
                using var stream = IoRetry.Run(() => new FileStream(path, FileMode.Open, FileAccess.Read, FileShare.Read), IoRetry.IsFileBusy, Delays, _ => holder.Dispose(), Ct);

                Assert.Equal(3, stream.Length);
            }
            finally
            {
                File.Delete(path);
            }
        }

        [Fact]
        public void CancellationEndsTheRetries()
        {
            using var cts = new CancellationTokenSource();
            cts.Cancel();

            Assert.Throws<OperationCanceledException>(() => IoRetry.Run<int>(() => throw Busy(), IoRetry.IsFileBusy, Delays, _ => { }, cts.Token));
        }
    }
}
