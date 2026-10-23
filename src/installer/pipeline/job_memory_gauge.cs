using System.ComponentModel;
using AssetConverter.Interop;

namespace AssetConverter.Pipeline
{
    /// <summary>
    /// Reads the memory used by a Job Object through the primary reading and, when the system reports that it does not support it, switches
    /// once, logged, to the fallback for good. The fallback reads are cached briefly, since they cost a query per process of the job.
    /// </summary>
    internal sealed class JobMemoryGauge
    {
        /// <summary>How long a fallback reading is reused.</summary>
        internal static readonly TimeSpan FallbackCacheDuration = TimeSpan.FromMilliseconds(100);

        private readonly Func<long> _primary;
        private readonly Func<long> _fallback;
        private readonly Action<string> _log;
        private readonly TimeProvider _clock;
        private readonly object _lock = new();
        private bool _usingFallback;
        private long? _cachedTimestamp;
        private long _cachedBytes;

        public JobMemoryGauge(Func<long> primary, Func<long> fallback, Action<string> log, TimeProvider? clock = null)
        {
            _primary = primary;
            _fallback = fallback;
            _log = log;
            _clock = clock ?? TimeProvider.System;
        }

        /// <exception cref="Win32Exception">The primary reading failed for a reason other than being unsupported, or the fallback failed.</exception>
        public long Read()
        {
            if (!Volatile.Read(ref _usingFallback))
            {
                try
                {
                    return _primary();
                }
                catch (Win32Exception exception) when (IsUnsupported(exception))
                {
                    SwitchToFallback(exception);
                }
            }

            return ReadFallbackCached();
        }

        private static bool IsUnsupported(Win32Exception exception) =>
            exception.NativeErrorCode is JobObjectNative.ErrorInvalidParameter or JobObjectNative.ErrorNotSupported;

        private void SwitchToFallback(Win32Exception exception)
        {
            lock (_lock)
            {
                if (_usingFallback)
                    return;
                _usingFallback = true;
            }

            _log($"Memory: the system does not report the job's memory usage directly ({exception.Message}); summing the private bytes of its processes instead");
        }

        private long ReadFallbackCached()
        {
            lock (_lock)
            {
                long now = _clock.GetTimestamp();
                if (_cachedTimestamp is { } cached && _clock.GetElapsedTime(cached, now) < FallbackCacheDuration)
                    return _cachedBytes;

                _cachedBytes = _fallback();
                _cachedTimestamp = now;
                return _cachedBytes;
            }
        }
    }
}
