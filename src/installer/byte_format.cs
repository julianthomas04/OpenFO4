using System.Globalization;

namespace AssetConverter
{
    public static class ByteFormat
    {
        public static string Format(long bytes) =>
            bytes >= 1L << 30 ? Scaled(bytes, 1L << 30, "GB") :
            bytes >= 1L << 20 ? Scaled(bytes, 1L << 20, "MB") :
            bytes >= 1L << 10 ? Scaled(bytes, 1L << 10, "KB") :
                                $"{bytes} B";

        static string Scaled(long bytes, long unit, string suffix) =>
            $"{(bytes / (double)unit).ToString("F1", CultureInfo.InvariantCulture)} {suffix}";
    }
}
