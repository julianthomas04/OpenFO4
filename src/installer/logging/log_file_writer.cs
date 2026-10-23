using System.Text;

namespace AssetConverter.Logging
{
    /// <summary>
    /// Writes every line to a log file, replacing any previous content, and flushes after each line so a crash keeps the log.
    /// A failing disk disables the file and is reported once through <c>onFault</c>; the installer keeps running. Lines written
    /// after <see cref="Dispose"/> are ignored.
    /// </summary>
    public sealed class LogFileWriter : ILineSink, IDisposable
    {
        private readonly object _gate = new();
        private readonly StreamWriter _writer;
        private readonly TimeProvider _clock;
        private readonly Action<string> _onFault;
        private bool _faulted;
        private bool _disposed;

        /// <exception cref="IOException">The file cannot be created or is held open by another process.</exception>
        /// <exception cref="UnauthorizedAccessException">The file or its folder is not writable.</exception>
        public LogFileWriter(string path, Action<string> onFault, TimeProvider? clock = null)
            : this(OpenFile(path), path, onFault, clock)
        {
        }

        internal LogFileWriter(Stream stream, string path, Action<string> onFault, TimeProvider? clock = null)
        {
            ArgumentNullException.ThrowIfNull(onFault);
            _onFault = onFault;
            _clock = clock ?? TimeProvider.System;
            Path = path;
            _writer = new StreamWriter(stream, new UTF8Encoding(encoderShouldEmitUTF8Identifier: false)) { AutoFlush = true };
        }

        public string Path { get; }

        public void WriteLine(string text, bool isError)
        {
            lock (_gate)
            {
                if (_faulted || _disposed) return;
                try
                {
                    _writer.WriteLine(new LogLine(0, _clock.GetUtcNow(), isError, text).Format());
                }
                catch (Exception ex) when (ex is IOException or UnauthorizedAccessException)
                {
                    Fault(ex);
                }
            }
        }

        public void Dispose()
        {
            lock (_gate)
            {
                if (_disposed) return;
                _disposed = true;
                try
                {
                    _writer.Dispose();
                }
                catch (Exception ex) when (ex is IOException or UnauthorizedAccessException)
                {
                    Fault(ex);
                }
            }
        }

        private void Fault(Exception ex)
        {
            if (_faulted) return;
            _faulted = true;
            _onFault($"WARNING: writing {Path} failed ({ex.Message}); the log file is disabled for the rest of the run.");
        }

        private static FileStream OpenFile(string path)
        {
            Directory.CreateDirectory(System.IO.Path.GetDirectoryName(System.IO.Path.GetFullPath(path))!);
            return new FileStream(path, FileMode.Create, FileAccess.Write, FileShare.Read);
        }
    }
}
