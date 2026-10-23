namespace AssetConverter
{
    /// <summary>
    /// Sees the exact bytes <see cref="BA2ParallelUnpacker"/> writes to the file of one general (non-texture) archive entry, in file order and
    /// in chunks of any size, while they are in memory anyway. An entry that fails part-way never reaches <see cref="Complete"/>.
    /// </summary>
    public interface IEntryContentObserver
    {
        void Write(ReadOnlySpan<byte> chunk);

        /// <summary>Every byte of the file was handed to it.</summary>
        void Complete();
    }

    /// <summary>A write-only stream that passes every write to <paramref name="inner"/> and then shows the accepted bytes to <paramref name="observer"/>.</summary>
    internal sealed class ObservedWriteStream(Stream inner, IEntryContentObserver observer) : Stream
    {
        public override bool CanRead => false;
        public override bool CanSeek => false;
        public override bool CanWrite => true;
        public override long Length => throw new NotSupportedException();

        public override long Position
        {
            get => throw new NotSupportedException();
            set => throw new NotSupportedException();
        }

        public override void Write(byte[] buffer, int offset, int count) => Write(buffer.AsSpan(offset, count));

        public override void Write(ReadOnlySpan<byte> buffer)
        {
            inner.Write(buffer);
            observer.Write(buffer);
        }

        public override void Flush() => inner.Flush();

        public override int Read(byte[] buffer, int offset, int count) => throw new NotSupportedException();

        public override long Seek(long offset, SeekOrigin origin) => throw new NotSupportedException();

        public override void SetLength(long value) => throw new NotSupportedException();
    }
}
