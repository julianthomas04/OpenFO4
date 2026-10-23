namespace AssetConverter
{
    /// <summary>
    /// The runtime faults a reader of game data raises when a value is not what it was written for: an overflow, an index or key that
    /// is not there, a cast or state that does not hold. A conversion step catches these to name the game file it was reading and keeps
    /// the fault as the inner exception.
    /// </summary>
    internal static class UnexpectedFault
    {
        public static bool Is(Exception e) =>
            e is OverflowException or ArgumentException or KeyNotFoundException or IndexOutOfRangeException or InvalidCastException
                or InvalidOperationException or EndOfStreamException or NullReferenceException or FormatException or DivideByZeroException;
    }
}
