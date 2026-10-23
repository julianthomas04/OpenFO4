using System.Collections.Concurrent;
using System.Text;

namespace AssetConverter
{
    /// <summary>
    /// Which staged NIFs the collision worker has to open. A NIF yields a sidecar only if its bytes contain the block-type name
    /// <see cref="PhysicsSystemMarker"/> (the worker's own prefilter, <c>_may_contain_collision</c>, applies this same rule to the whole file),
    /// so the extraction tells the NIFs it writes apart while their bytes are still in memory and the worker is spared the open of the rest.
    /// A NIF nobody observed (left by an earlier run, or whose extraction failed part-way) is always a candidate: the worker decides it.
    /// </summary>
    internal sealed class CollisionCandidates
    {
        internal const string PhysicsSystemMarker = "bhkPhysicsSystem";

        private static readonly byte[] MarkerBytes = Encoding.ASCII.GetBytes(PhysicsSystemMarker);

        private readonly ConcurrentDictionary<string, bool> _hasMarker = new(StringComparer.OrdinalIgnoreCase);

        /// <summary>The observer for the NIF written to <paramref name="path"/>; when the file is complete its verdict is recorded under <paramref name="path"/>.</summary>
        public IEntryContentObserver Observe(string path) => new MarkerObserver(this, path);

        /// <returns>Whether the NIF written to <paramref name="path"/> was seen complete with the marker, so it is a candidate no matter what else is known about the staged files.</returns>
        public bool HasMarker(string path) => _hasMarker.TryGetValue(path, out bool hasMarker) && hasMarker;

        /// <returns>The NIFs of <paramref name="stagedNifs"/> (every NIF on disk) that are not known to lack the marker, in path order.</returns>
        public string[] Select(IEnumerable<string> stagedNifs) =>
            [.. stagedNifs.Where(path => !_hasMarker.TryGetValue(path, out bool hasMarker) || hasMarker).Order(StringComparer.OrdinalIgnoreCase)];

        private sealed class MarkerObserver(CollisionCandidates owner, string path) : IEntryContentObserver
        {
            private readonly StreamingByteSearch _search = new(MarkerBytes);

            public void Write(ReadOnlySpan<byte> chunk) => _search.Feed(chunk);

            public void Complete() => owner._hasMarker[path] = _search.Found;
        }
    }
}
