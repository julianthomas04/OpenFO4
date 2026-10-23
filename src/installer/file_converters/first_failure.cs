namespace AssetConverter
{
    /// <summary>Keeps the first failure of the concurrent workers of a conversion, stops the rest and reports it with the number of the others as one exception of the conversion's own type.</summary>
    /// <param name="create">Makes the conversion's exception from a message and its cause.</param>
    /// <param name="unitPlural">What the workers convert, for the count of further failures.</param>
    internal sealed class FirstFailure<TException>(CancellationTokenSource stop, Func<string, Exception?, TException> create, string unitPlural) where TException : Exception
    {
        private readonly object _gate = new();
        private Exception? _first;
        private int _count;

        public void Record(Exception error)
        {
            lock (_gate)
            {
                _first ??= error;
                _count++;
            }
            stop.Cancel();
        }

        public void ThrowIfAny()
        {
            lock (_gate)
            {
                if (_first is null) return;
                string others = _count > 1 ? $" ({_count - 1:N0} more {unitPlural} failed)" : "";
                throw _first is TException known
                    ? create(known.Message + others, known.InnerException)
                    : create(_first.Message + others, _first);
            }
        }
    }
}
