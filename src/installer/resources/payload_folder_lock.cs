using System.Security.Cryptography;
using System.Text;

namespace AssetConverter
{
    /// <summary>
    /// The system-wide lock on one payload folder: whoever unpacks it, replaces it or deletes it holds the lock, so installers (of any
    /// version) never work on the same folder at once. The lock is a named mutex, so it ends with the process that holds it.
    /// </summary>
    internal sealed class PayloadFolderLock : IDisposable
    {
        private readonly Mutex _mutex;

        private PayloadFolderLock(Mutex mutex) => _mutex = mutex;

        /// <exception cref="TimeoutException">Another installer holds the lock for longer than <paramref name="timeout"/>.</exception>
        public static PayloadFolderLock Acquire(string folder, TimeSpan timeout)
        {
            var mutex = new Mutex(false, MutexName(folder));
            try
            {
                if (!mutex.WaitOne(timeout)) throw new TimeoutException("another installer is still working on the same files");
            }
            catch (AbandonedMutexException)
            {
                // The other installer ended without finishing; the completion marker tells whether its work is usable.
            }
            catch
            {
                mutex.Dispose();
                throw;
            }
            return new PayloadFolderLock(mutex);
        }

        /// <returns>The lock, or null when another installer holds it at this moment.</returns>
        public static PayloadFolderLock? TryAcquire(string folder)
        {
            try
            {
                return Acquire(folder, TimeSpan.Zero);
            }
            catch (TimeoutException)
            {
                return null;
            }
        }

        public void Dispose()
        {
            _mutex.ReleaseMutex();
            _mutex.Dispose();
        }

        private static string MutexName(string folder) =>
            @"Local\OpenFO4.Payload." + Convert.ToHexString(SHA256.HashData(Encoding.UTF8.GetBytes(folder.ToUpperInvariant())))[..32];
    }
}
