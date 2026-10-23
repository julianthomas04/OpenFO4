namespace AssetConverter
{
    /// <summary>
    /// Walks the records of the listed types in a plugin's raw bytes (descending into groups, inflating compressed
    /// records) and hands each record's subrecord bytes to a decoder. The raw scans use it for data Mutagen does not
    /// expose for Fallout 4. Each record is keyed by its resolved form key.
    /// </summary>
    internal static class RawEsmScanner
    {
        const int RecordHeaderSize = 24;
        const int SubrecordHeaderSize = 6;
        const int CompressedFlag = 0x00040000;
        const string ExtendedSizeSignature = "XXXX";

        /// <summary>
        /// Adds a decoded entry to <paramref name="into"/> for every record of <paramref name="recordTypes"/> the decoder
        /// accepts (returns non-null). Entries found before a failure stay in <paramref name="into"/>.
        /// </summary>
        public static void Scan<TResult>(string esmPath, string scanName, IReadOnlySet<string> recordTypes,
            Func<string, byte[], TResult?> decode, Dictionary<string, TResult> into)
            where TResult : class
        {
            var data = File.ReadAllBytes(esmPath);
            var masters = CellProcessor.ReadFormIdPluginTable(data, Path.GetFileName(esmPath));
            if (data.Length < RecordHeaderSize) return;
            int tes4DataSize = BitConverter.ToInt32(data, 4);
            ScanGroup(data, RecordHeaderSize + tes4DataSize, data.Length, masters, scanName, recordTypes, decode, into);
        }

        public static IEnumerable<(string Signature, byte[] Body)> Subrecords(byte[] recordData)
        {
            int i = 0;
            int extendedSize = 0;
            while (i + SubrecordHeaderSize <= recordData.Length)
            {
                string signature = System.Text.Encoding.ASCII.GetString(recordData, i, 4);
                int size = extendedSize > 0 ? extendedSize : BitConverter.ToUInt16(recordData, i + 4);
                extendedSize = 0;
                int start = i + SubrecordHeaderSize;
                if (start + size > recordData.Length) yield break;
                if (signature == ExtendedSizeSignature && size == 4) extendedSize = BitConverter.ToInt32(recordData, start);
                else yield return (signature, recordData[start..(start + size)]);
                i = start + size;
            }
        }

        static void ScanGroup<TResult>(byte[] data, int pos, int end, List<string> masters, string scanName,
            IReadOnlySet<string> recordTypes, Func<string, byte[], TResult?> decode, Dictionary<string, TResult> into)
            where TResult : class
        {
            while (pos + RecordHeaderSize <= end)
            {
                string signature = System.Text.Encoding.ASCII.GetString(data, pos, 4);
                if (signature == "GRUP")
                {
                    int groupSize = BitConverter.ToInt32(data, pos + 4);
                    if (groupSize < RecordHeaderSize || pos + groupSize > end) { pos += RecordHeaderSize; continue; }
                    ScanGroup(data, pos + RecordHeaderSize, pos + groupSize, masters, scanName, recordTypes, decode, into);
                    pos += groupSize;
                    continue;
                }

                int recordEnd = pos + RecordHeaderSize + BitConverter.ToInt32(data, pos + 4);
                if (recordEnd > end) break;
                if (recordTypes.Contains(signature)) ScanRecord(data, pos, recordEnd, masters, scanName, decode, into);
                pos = recordEnd;
            }
        }

        static void ScanRecord<TResult>(byte[] data, int pos, int recordEnd, List<string> masters, string scanName,
            Func<string, byte[], TResult?> decode, Dictionary<string, TResult> into)
            where TResult : class
        {
            string formKey = CellProcessor.ResolveFormKey(BitConverter.ToUInt32(data, pos + 12), masters);
            byte[]? recordData;
            if ((BitConverter.ToInt32(data, pos + 8) & CompressedFlag) != 0)
            {
                if (!CellProcessor.TryInflateRecord(data, pos, recordEnd, scanName, out recordData)) return;
            }
            else
            {
                recordData = data[(pos + RecordHeaderSize)..recordEnd];
            }

            var decoded = decode(formKey, recordData);
            if (decoded != null) into[formKey] = decoded;
        }
    }
}
