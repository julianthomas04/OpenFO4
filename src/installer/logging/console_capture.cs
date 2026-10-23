namespace AssetConverter.Logging
{
    /// <summary>Replaces <see cref="Console.Out"/> and <see cref="Console.Error"/> with tee writers for its lifetime, then restores the originals.</summary>
    public sealed class ConsoleCapture : IDisposable
    {
        private readonly TextWriter _originalOut;
        private readonly TextWriter _originalError;
        private readonly TeeTextWriter _out;
        private readonly TeeTextWriter _error;
        private bool _disposed;

        private ConsoleCapture(ILineSink sink)
        {
            _originalOut = Console.Out;
            _originalError = Console.Error;
            _out = new TeeTextWriter(_originalOut, sink, isError: false);
            _error = new TeeTextWriter(_originalError, sink, isError: true);
            Console.SetOut(_out);
            Console.SetError(_error);
        }

        public static ConsoleCapture Start(ILineSink sink) => new(sink);

        /// <summary>Where a sink fault message goes without passing through the capture again.</summary>
        public TextWriter OriginalError => _originalError;

        public void Dispose()
        {
            if (_disposed) return;
            _disposed = true;
            Console.SetOut(_originalOut);
            Console.SetError(_originalError);
            _out.Dispose();
            _error.Dispose();
        }
    }
}
