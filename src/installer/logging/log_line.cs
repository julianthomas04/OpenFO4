using System.Globalization;

namespace AssetConverter.Logging
{
    /// <summary>One line of installer output. <see cref="Sequence"/> is 1-based and increases by one per line of a <see cref="LogBuffer"/>.</summary>
    public sealed record LogLine(long Sequence, DateTimeOffset Timestamp, bool IsError, string Text)
    {
        public string Format() => string.Create(CultureInfo.InvariantCulture, $"{Timestamp.LocalDateTime:HH:mm:ss.fff} {(IsError ? "ERR" : "   ")} {Text}");
    }

    /// <summary>Receives completed lines of console output.</summary>
    public interface ILineSink
    {
        void WriteLine(string text, bool isError);
    }
}
