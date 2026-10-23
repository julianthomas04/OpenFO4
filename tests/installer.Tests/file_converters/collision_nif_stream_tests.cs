using System.Text;
using AssetConverter;
using Xunit;

namespace InstallerTests
{
    /// <summary>The live form of the collision worker's list: what is written to its stdin while the NIFs are still being queued.</summary>
    public sealed class CollisionNifStreamTests : IDisposable
    {
        private const string Root = @"C:\staging\Meshes";

        private static readonly TimeSpan Patience = TimeSpan.FromSeconds(10);

        private readonly MemoryStream _bytes = new();

        public void Dispose() => _bytes.Dispose();

        private static CancellationToken Ct => TestContext.Current.CancellationToken;

        private static string Nif(string relative) => Path.Combine(Root, relative);

        private string Written() => Encoding.UTF8.GetString(_bytes.ToArray());

        private int StreamAll(NifWorkQueue queue, Action? onTaken = null)
        {
            using var writer = CollisionNifList.OpenWriter(_bytes);
            return CollisionNifList.StreamTo(writer, Root, queue, onTaken, Ct);
        }

        private static NifWorkQueue Finished(params string[] relatives)
        {
            var queue = new NifWorkQueue();
            foreach (string relative in relatives) queue.Add(Nif(relative));
            queue.Complete();
            return queue;
        }

        [Fact]
        public void EveryQueuedFileIsWrittenOnceAsItsRelativePathInQueueOrder()
        {
            int written = StreamAll(Finished(@"b\two.nif", @"a\one.nif", "three.nif"));

            Assert.Equal(3, written);
            Assert.Equal("b\\two.nif\na\\one.nif\nthree.nif\n", Written());
        }

        [Fact]
        public void TheLinesAreUtf8WithoutByteOrderMarkEndedByLineFeedsLikeTheListFile()
        {
            StreamAll(Finished(@"Ünïcode\Gebäude Süd.nif", @"日本語\モデル.nif"));

            Assert.Equal(Encoding.UTF8.GetBytes("Ünïcode\\Gebäude Süd.nif\n日本語\\モデル.nif\n"), _bytes.ToArray());
        }

        [Fact]
        public void TheFileFormatAndTheStreamFormatAreByteIdenticalForTheSameFiles()
        {
            string[] relatives = [@"a\one.nif", @"ä\two.nif"];
            StreamAll(Finished(relatives));
            string listPath = Path.GetTempFileName();
            try
            {
                CollisionNifList.WriteTo(listPath, Root, relatives.Select(Nif));

                Assert.Equal(File.ReadAllBytes(listPath), _bytes.ToArray());
            }
            finally
            {
                File.Delete(listPath);
            }
        }

        [Fact]
        public void AFinishedEmptyQueueWritesNothing()
        {
            Assert.Equal(0, StreamAll(Finished()));
            Assert.Empty(_bytes.ToArray());
        }

        [Fact]
        public void OnTakenIsCalledOncePerLine()
        {
            int taken = 0;

            StreamAll(Finished("a.nif", "b.nif", "c.nif"), () => taken++);

            Assert.Equal(3, taken);
        }

        [Fact]
        public async Task ALineReachesTheDestinationAsSoonAsItsFileIsQueuedWhileTheQueueIsStillOpen()
        {
            var queue = new NifWorkQueue();
            using var writer = CollisionNifList.OpenWriter(_bytes);
            var streaming = Task.Run(() => CollisionNifList.StreamTo(writer, Root, queue, null, Ct), Ct);

            queue.Add(Nif("early.nif"));
            var deadline = DateTime.UtcNow + Patience;
            while (_bytes.Length == 0 && DateTime.UtcNow < deadline) await Task.Delay(5, Ct);
            string beforeTheEnd = Written();
            queue.Add(Nif("late.nif"));
            queue.Complete();

            Assert.Equal(2, await streaming.WaitAsync(Patience, Ct));
            Assert.Equal("early.nif\n", beforeTheEnd);
            Assert.Equal("early.nif\nlate.nif\n", Written());
        }

        [Fact]
        public void ThePathOfAFileBelowTheRootKeepsItsFolders()
        {
            StreamAll(Finished(@"actors\dog\walk.nif"));

            Assert.Equal("actors\\dog\\walk.nif\n", Written());
        }

        [Fact]
        public void AFileOutsideTheSourceRootIsRejectedAndNothingIsWrittenForIt()
        {
            var queue = new NifWorkQueue();
            queue.Add(Nif("ok.nif"));
            queue.Add(@"C:\elsewhere\x.nif");
            queue.Complete();

            Assert.Throws<ArgumentException>(() => StreamAll(queue));

            Assert.Equal("ok.nif\n", Written());
        }

        [Fact]
        public void AFailedQueueRaisesItsExceptionAtOnceEvenWhileFilesQueuedBeforeItAreStillWaiting()
        {
            var queue = new NifWorkQueue();
            queue.Add(Nif("before.nif"));
            var cause = new InvalidOperationException("extraction failed");
            queue.Fail(cause);

            var raised = Assert.Throws<InvalidOperationException>(() => StreamAll(queue));

            Assert.Same(cause, raised);
            Assert.Equal("", Written());
        }

        [Fact]
        public void CancellingWhileWaitingForTheNextFileEndsTheStreamWithTheCancellation()
        {
            using var cancel = new CancellationTokenSource();
            using var writer = CollisionNifList.OpenWriter(_bytes);
            var queue = new NifWorkQueue();
            cancel.CancelAfter(TimeSpan.FromMilliseconds(100));

            Assert.ThrowsAny<OperationCanceledException>(() => CollisionNifList.StreamTo(writer, Root, queue, null, cancel.Token));
        }

        [Fact]
        public void AFailedWriteToTheReaderSurfacesTheIoException()
        {
            using var writer = CollisionNifList.OpenWriter(new BrokenPipeStream());

            Assert.Throws<IOException>(() => CollisionNifList.StreamTo(writer, Root, Finished("a.nif"), null, Ct));
        }

        /// <summary>Fails its first write the way a pipe whose reader has exited does, then takes whatever is left (the disposal of the writer).</summary>
        private sealed class BrokenPipeStream : Stream
        {
            private bool _failed;

            public override bool CanRead => false;

            public override bool CanSeek => false;

            public override bool CanWrite => true;

            public override long Length => throw new NotSupportedException();

            public override long Position
            {
                get => throw new NotSupportedException();
                set => throw new NotSupportedException();
            }

            public override void Flush() { }

            public override int Read(byte[] buffer, int offset, int count) => throw new NotSupportedException();

            public override long Seek(long offset, SeekOrigin origin) => throw new NotSupportedException();

            public override void SetLength(long value) => throw new NotSupportedException();

            public override void Write(byte[] buffer, int offset, int count)
            {
                if (_failed) return;
                _failed = true;
                throw new IOException("The pipe has been ended.");
            }
        }
    }
}
