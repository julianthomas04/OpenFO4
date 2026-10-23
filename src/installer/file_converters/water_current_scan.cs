using System.Globalization;
using System.Text.Json.Nodes;

namespace AssetConverter
{
    /// <summary>
    /// Raw scan of the water current blocks of CELL and REFR records: <c>XWCN</c> (u32 velocity count) followed by
    /// <c>XWCU</c> (that many 16-byte entries: float x, y, z velocity and one more float). Mutagen models the block as a
    /// fixed 32-byte struct, which is not the layout xEdit documents for Fallout 4, so the bytes are decoded here.
    /// Each record with a block maps to a JSON array of <c>[x,y,z,w]</c> entries. A block whose <c>XWCU</c> size does
    /// not match its <c>XWCN</c> count is tallied in <see cref="ReadFailureTally"/> and left out.
    /// </summary>
    internal static class WaterCurrentScan
    {
        public const int EntrySize = 16;
        const string CountSignature = "XWCN";
        const string CurrentsSignature = "XWCU";
        const string FailureSite = "XWCU water currents";

        static readonly HashSet<string> RecordTypes = new(StringComparer.Ordinal) { "CELL", "REFR" };

        public static Dictionary<string, string> Scan(string esmPath)
        {
            var result = new Dictionary<string, string>(StringComparer.OrdinalIgnoreCase);
            try
            {
                RawEsmScanner.Scan(esmPath, "water current raw scan", RecordTypes,
                    (formKey, recordData) => Decode(recordData, formKey)?.ToJsonString(), result);
            }
            catch (Exception ex) when (ex is IOException or InvalidDataException)
            {
                Console.WriteLine($"[WARN] water current raw scan failed, its data is left out: {ex.GetType().Name}: {ex.Message}");
            }
            return result;
        }

        /// <summary>Decodes the entries of one record's subrecords; null when the record has no current block.</summary>
        internal static JsonArray? Decode(byte[] recordData, string subject)
        {
            uint? count = null;
            byte[]? currents = null;
            foreach (var (signature, body) in RawEsmScanner.Subrecords(recordData))
            {
                if (signature == CountSignature && body.Length == 4) count = BitConverter.ToUInt32(body);
                else if (signature == CurrentsSignature) currents = body;
            }
            if (count == null && currents == null) return null;
            if (count == null || currents == null || (long)count.Value * EntrySize != currents.Length)
            {
                ReadFailureTally.Shared.Record(FailureSite,
                    new InvalidDataException(string.Create(CultureInfo.InvariantCulture, $"XWCN count {count?.ToString(CultureInfo.InvariantCulture) ?? "missing"} does not match XWCU of {currents?.Length.ToString(CultureInfo.InvariantCulture) ?? "missing"} bytes")), subject);
                return null;
            }
            var entries = new JsonArray();
            for (int i = 0; i < currents.Length; i += EntrySize)
                entries.Add(new JsonArray(
                    BitConverter.ToSingle(currents, i), BitConverter.ToSingle(currents, i + 4),
                    BitConverter.ToSingle(currents, i + 8), BitConverter.ToSingle(currents, i + 12)));
            return entries;
        }
    }
}
