namespace AssetConverter.Logging
{
    /// <summary>Forwards each line to every sink under one lock, so all sinks see the same line order.</summary>
    public sealed class CompositeLineSink : ILineSink
    {
        private readonly object _gate = new();
        private readonly ILineSink[] _sinks;

        public CompositeLineSink(params ILineSink[] sinks)
        {
            ArgumentNullException.ThrowIfNull(sinks);
            _sinks = sinks;
        }

        public void WriteLine(string text, bool isError)
        {
            lock (_gate)
            {
                foreach (var sink in _sinks) sink.WriteLine(text, isError);
            }
        }
    }
}
