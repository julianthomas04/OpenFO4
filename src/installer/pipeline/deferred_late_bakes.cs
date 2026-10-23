namespace AssetConverter.Pipeline
{
    /// <summary>
    /// Palette bakes the ESM load discovers (MSWP color remapping) need the extracted textures, which are still arriving while the
    /// database is built. While this is alive the load queues those bakes in <see cref="PaletteBaker"/> instead of baking them;
    /// <see cref="Drain"/> bakes the queue once the load has finished and nothing can add to it any more.
    /// </summary>
    internal sealed class DeferredLateBakes : IDisposable
    {
        private bool _drained;

        public DeferredLateBakes()
        {
            PaletteBaker.DeferLateBakes = true;
        }

        /// <param name="databaseLoaded">The ESM load, which must have succeeded: it is the only producer of queued bakes.</param>
        /// <returns>The number of bakes that were queued.</returns>
        public int Drain(Task databaseLoaded, string stagingRoot, string texturesOut)
        {
            if (!databaseLoaded.IsCompletedSuccessfully)
                throw new InvalidOperationException("The deferred palette bakes can only be drained once the database load has finished.");
            if (_drained)
                throw new InvalidOperationException("The deferred palette bakes were already drained.");
            _drained = true;
            PaletteBaker.DeferLateBakes = false;
            return PaletteBaker.RunDeferredLateBakes(stagingRoot, texturesOut);
        }

        public void Dispose()
        {
            PaletteBaker.DeferLateBakes = false;
        }
    }
}
