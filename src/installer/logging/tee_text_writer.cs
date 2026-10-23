using System.Text;

namespace AssetConverter.Logging
{
    /// <summary>
    /// A console writer that hands each completed line to an <see cref="ILineSink"/> and passes everything through to the original
    /// writer unchanged. The pass-through runs on its own thread, in order, so a console that stops accepting output (selection mode,
    /// a full pipe) never delays the sink; <see cref="Flush"/> and disposing wait for it, each for at most the stall timeout, and say so when it ran out. Text without a trailing newline stays
    /// pending for the sink until the line completes or the writer is disposed.
    /// </summary>
    public sealed class TeeTextWriter : TextWriter
    {
        public const int DefaultConsoleQueueCapacity = 50_000;
        public static readonly TimeSpan DefaultStallTimeout = TimeSpan.FromSeconds(5);

        private readonly object _gate = new();
        private readonly TextWriter _inner;
        private readonly ILineSink _sink;
        private readonly bool _isError;
        private readonly StringBuilder _pending = new();
        private readonly BackgroundTextWriter _console;
        private readonly TimeSpan _stallTimeout;
        private int _stallReported;

        public TeeTextWriter(TextWriter inner, ILineSink sink, bool isError, int consoleQueueCapacity = DefaultConsoleQueueCapacity, TimeSpan? stallTimeout = null)
        {
            ArgumentNullException.ThrowIfNull(inner);
            ArgumentNullException.ThrowIfNull(sink);
            _inner = inner;
            _sink = sink;
            _isError = isError;
            _stallTimeout = stallTimeout ?? DefaultStallTimeout;
            NewLine = inner.NewLine;
            _console = new BackgroundTextWriter(inner, consoleQueueCapacity, message => sink.WriteLine(message, true));
        }

        public override Encoding Encoding => _inner.Encoding;

        public override IFormatProvider FormatProvider => _inner.FormatProvider;

        public override void Write(char value) => Append(new ReadOnlySpan<char>(in value));

        public override void Write(char[] buffer, int index, int count) => Append(buffer.AsSpan(index, count));

        public override void Write(string? value)
        {
            if (value != null) Append(value);
        }

        public override void Write(ReadOnlySpan<char> buffer) => Append(buffer);

        public override void WriteLine() => Append(NewLine);

        public override void WriteLine(string? value) => Append(string.Concat(value, NewLine));

        public override void WriteLine(ReadOnlySpan<char> buffer) => Append(string.Concat(buffer, NewLine));

        /// <summary>
        /// Returns once everything written so far has reached the original writer and it has been flushed, or after the stall timeout;
        /// the first timeout of a stall is reported to the sink, and the text stays queued for the console.
        /// </summary>
        public override void Flush()
        {
            if (_console.Flush(_stallTimeout))
            {
                Volatile.Write(ref _stallReported, 0);
                return;
            }
            if (Interlocked.Exchange(ref _stallReported, 1) == 0)
                _sink.WriteLine($"WARNING: the console did not take its output within {_stallTimeout.TotalSeconds:0.##} s; flushing gave up waiting, the text stays queued for it and is in the log file.", true);
        }

        protected override void Dispose(bool disposing)
        {
            if (disposing)
            {
                FlushPendingLine();
                if (!_console.Close(_stallTimeout))
                    _sink.WriteLine($"WARNING: the console did not take its last output within {_stallTimeout.TotalSeconds:0.##} s; it is only in the log file.", true);
            }
            base.Dispose(disposing);
        }

        private void Append(ReadOnlySpan<char> text)
        {
            lock (_gate)
            {
                _console.Write(text.ToString());
                int start = 0;
                for (int i = 0; i < text.Length; i++)
                {
                    if (text[i] != '\n') continue;
                    _pending.Append(text[start..i]);
                    EmitPendingLine();
                    start = i + 1;
                }
                _pending.Append(text[start..]);
            }
        }

        private void FlushPendingLine()
        {
            lock (_gate)
            {
                if (_pending.Length > 0) EmitPendingLine();
            }
        }

        private void EmitPendingLine()
        {
            if (_pending.Length > 0 && _pending[^1] == '\r') _pending.Length--;
            string line = _pending.ToString();
            _pending.Clear();
            _sink.WriteLine(line, _isError);
        }
    }
}
