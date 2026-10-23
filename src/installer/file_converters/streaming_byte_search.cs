namespace AssetConverter
{
    /// <summary>Whether a byte pattern occurs in a byte sequence delivered in chunks of any size, matches across chunk boundaries included.</summary>
    internal sealed class StreamingByteSearch
    {
        private readonly byte[] _pattern;
        private readonly byte[] _tail;
        private int _tailLength;

        public StreamingByteSearch(ReadOnlySpan<byte> pattern)
        {
            if (pattern.IsEmpty) throw new ArgumentException("The pattern must not be empty.", nameof(pattern));
            _pattern = pattern.ToArray();
            _tail = new byte[pattern.Length - 1];
        }

        public bool Found { get; private set; }

        public void Feed(ReadOnlySpan<byte> chunk)
        {
            if (Found || chunk.IsEmpty) return;
            if (OccursAcrossBoundary(chunk) || chunk.IndexOf(_pattern) >= 0)
            {
                Found = true;
                return;
            }
            KeepTail(chunk);
        }

        /// <summary>A match that starts in the kept tail of the earlier chunks needs at most pattern-1 bytes of this chunk.</summary>
        private bool OccursAcrossBoundary(ReadOnlySpan<byte> chunk)
        {
            if (_tailLength == 0) return false;
            int head = Math.Min(chunk.Length, _pattern.Length - 1);
            Span<byte> window = stackalloc byte[_tailLength + head];
            _tail.AsSpan(0, _tailLength).CopyTo(window);
            chunk[..head].CopyTo(window[_tailLength..]);
            return window.IndexOf(_pattern) >= 0;
        }

        private void KeepTail(ReadOnlySpan<byte> chunk)
        {
            int keep = Math.Min(_tail.Length, _tailLength + chunk.Length);
            int fromChunk = Math.Min(keep, chunk.Length);
            int fromOldTail = keep - fromChunk;
            _tail.AsSpan(_tailLength - fromOldTail, fromOldTail).CopyTo(_tail);
            chunk[^fromChunk..].CopyTo(_tail.AsSpan(fromOldTail));
            _tailLength = keep;
        }
    }
}
