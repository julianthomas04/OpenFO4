using System.IO.Compression;
using System.Text;

namespace InstallerTests
{
    /// <summary>
    /// Writes minimal binary plugin files by hand, following the record layout from the UESP / xEdit Fallout 4 format:
    /// 24-byte record header (signature, data size, flags, form id, version control, form version, unknown) followed
    /// by subrecords (signature, u16 size, data); GRUP headers are 24 bytes with the total group size including the header.
    /// Used for the raw scans that read bytes Mutagen does not expose (XCRI, XMSP, MODS, MODC).
    /// </summary>
    internal sealed class RawEsmBuilder
    {
        public const int CompressedFlag = 0x00040000;

        readonly List<byte> _bytes = [];

        RawEsmBuilder()
        {
        }

        public static RawEsmBuilder Plugin(params string[] masters)
        {
            var builder = new RawEsmBuilder();
            var header = new List<byte>();
            header.AddRange(Subrecord("HEDR", new byte[12]));
            foreach (var master in masters)
            {
                header.AddRange(Subrecord("MAST", Encoding.ASCII.GetBytes(master + "\0")));
                header.AddRange(Subrecord("DATA", new byte[8]));
            }
            builder.WriteRecord("TES4", 0, 0, header.ToArray());
            return builder;
        }

        public static byte[] Subrecord(string signature, byte[] data) =>
            Concat(Encoding.ASCII.GetBytes(signature), BitConverter.GetBytes((ushort)data.Length), data);

        public static byte[] FormIdSubrecord(string signature, uint formId) => Subrecord(signature, BitConverter.GetBytes(formId));

        public static byte[] FloatSubrecord(string signature, float value) => Subrecord(signature, BitConverter.GetBytes(value));

        public static byte[] Concat(params byte[][] parts) => parts.SelectMany(p => p).ToArray();

        public RawEsmBuilder Record(string signature, uint formId, params byte[][] subrecords)
        {
            WriteRecord(signature, 0, formId, Concat(subrecords));
            return this;
        }

        public RawEsmBuilder CompressedRecord(string signature, uint formId, params byte[][] subrecords)
        {
            byte[] raw = Concat(subrecords);
            using var compressed = new MemoryStream();
            using (var zlib = new ZLibStream(compressed, CompressionMode.Compress, leaveOpen: true))
                zlib.Write(raw);
            WriteRecord(signature, CompressedFlag, formId, Concat(BitConverter.GetBytes(raw.Length), compressed.ToArray()));
            return this;
        }

        /// <summary>A record flagged compressed whose body is not a valid zlib stream.</summary>
        public RawEsmBuilder CorruptCompressedRecord(string signature, uint formId)
        {
            WriteRecord(signature, CompressedFlag, formId, Concat(BitConverter.GetBytes(64), [0xDE, 0xAD, 0xBE, 0xEF, 0xDE, 0xAD]));
            return this;
        }

        /// <summary>Writes a group containing records produced by <paramref name="contents"/>; groups nest.</summary>
        public RawEsmBuilder Group(string label, Action<RawEsmBuilder> contents)
        {
            var inner = new RawEsmBuilder();
            contents(inner);
            _bytes.AddRange(Encoding.ASCII.GetBytes("GRUP"));
            _bytes.AddRange(BitConverter.GetBytes(24 + inner._bytes.Count));
            _bytes.AddRange(Encoding.ASCII.GetBytes(label.PadRight(4)[..4]));
            _bytes.AddRange(new byte[12]);
            _bytes.AddRange(inner._bytes);
            return this;
        }

        /// <summary>
        /// Sets major record flags on one record of an existing plugin file, in place. Mutagen strips the content of
        /// records it writes as deleted, so a deleted record that still carries data has to be flagged after writing.
        /// </summary>
        public static void AddRecordFlags(string path, string signature, uint localFormId, int flags)
        {
            byte[] file = File.ReadAllBytes(path);
            int masterCount = CountMasters(file);
            uint rawFormId = ((uint)masterCount << 24) | localFormId;
            byte[] signatureBytes = Encoding.ASCII.GetBytes(signature);
            for (int i = 0; i + 24 <= file.Length; i++)
            {
                if (!file.AsSpan(i, 4).SequenceEqual(signatureBytes) || BitConverter.ToUInt32(file, i + 12) != rawFormId) continue;
                BitConverter.GetBytes(BitConverter.ToInt32(file, i + 8) | flags).CopyTo(file, i + 8);
                File.WriteAllBytes(path, file);
                return;
            }
            throw new InvalidOperationException($"No {signature} record {localFormId:X6} found in {path}.");
        }

        /// <summary>
        /// Appends subrecords to one record of an existing plugin file, in place, growing the record and every group
        /// that encloses it. For subrecords Mutagen has no model for (it rewrites or drops them when writing).
        /// </summary>
        public static void AppendSubrecords(string path, string signature, uint localFormId, params byte[][] subrecords)
        {
            byte[] file = File.ReadAllBytes(path);
            uint rawFormId = ((uint)CountMasters(file) << 24) | localFormId;
            var enclosingGroups = new List<int>();
            int recordPos = FindRecord(file, 24 + BitConverter.ToInt32(file, 4), file.Length, Encoding.ASCII.GetBytes(signature), rawFormId, enclosingGroups)
                ?? throw new InvalidOperationException($"No {signature} record {localFormId:X6} found in {path}.");

            byte[] extra = Concat(subrecords);
            int recordEnd = recordPos + 24 + BitConverter.ToInt32(file, recordPos + 4);
            foreach (int offset in enclosingGroups.Append(recordPos))
                BitConverter.GetBytes(BitConverter.ToInt32(file, offset + 4) + extra.Length).CopyTo(file, offset + 4);
            File.WriteAllBytes(path, [.. file.AsSpan(0, recordEnd), .. extra, .. file.AsSpan(recordEnd)]);
        }

        /// <summary>
        /// Replaces the first subrecord named <paramref name="subSignature"/> of one record of an existing plugin file,
        /// in place, resizing the record and every group that encloses it. For subrecords whose exact bytes Mutagen
        /// does not preserve.
        /// </summary>
        public static void ReplaceSubrecord(string path, string signature, uint localFormId, string subSignature, byte[] data)
        {
            byte[] file = File.ReadAllBytes(path);
            uint rawFormId = ((uint)CountMasters(file) << 24) | localFormId;
            var enclosingGroups = new List<int>();
            int recordPos = FindRecord(file, 24 + BitConverter.ToInt32(file, 4), file.Length, Encoding.ASCII.GetBytes(signature), rawFormId, enclosingGroups)
                ?? throw new InvalidOperationException($"No {signature} record {localFormId:X6} found in {path}.");

            int recordEnd = recordPos + 24 + BitConverter.ToInt32(file, recordPos + 4);
            int subrecordPos = FindSubrecord(file, recordPos + 24, recordEnd, subSignature)
                ?? throw new InvalidOperationException($"{signature} record {localFormId:X6} has no {subSignature} subrecord in {path}.");
            int subrecordEnd = subrecordPos + 6 + BitConverter.ToUInt16(file, subrecordPos + 4);

            byte[] replacement = Subrecord(subSignature, data);
            int growth = replacement.Length - (subrecordEnd - subrecordPos);
            foreach (int offset in enclosingGroups.Append(recordPos))
                BitConverter.GetBytes(BitConverter.ToInt32(file, offset + 4) + growth).CopyTo(file, offset + 4);
            File.WriteAllBytes(path, [.. file.AsSpan(0, subrecordPos), .. replacement, .. file.AsSpan(subrecordEnd)]);
        }

        static int? FindSubrecord(byte[] file, int pos, int end, string signature)
        {
            while (pos + 6 <= end)
            {
                if (Encoding.ASCII.GetString(file, pos, 4) == signature) return pos;
                pos += 6 + BitConverter.ToUInt16(file, pos + 4);
            }
            return null;
        }

        static int? FindRecord(byte[] file, int pos, int end, byte[] signature, uint rawFormId, List<int> enclosingGroups)
        {
            while (pos + 24 <= end)
            {
                int size = BitConverter.ToInt32(file, pos + 4);
                if (file.AsSpan(pos, 4).SequenceEqual("GRUP"u8))
                {
                    enclosingGroups.Add(pos);
                    if (FindRecord(file, pos + 24, pos + size, signature, rawFormId, enclosingGroups) is { } found) return found;
                    enclosingGroups.RemoveAt(enclosingGroups.Count - 1);
                    pos += size;
                    continue;
                }
                if (file.AsSpan(pos, 4).SequenceEqual(signature) && BitConverter.ToUInt32(file, pos + 12) == rawFormId) return pos;
                pos += 24 + size;
            }
            return null;
        }

        static int CountMasters(byte[] file)
        {
            int end = 24 + BitConverter.ToInt32(file, 4);
            int count = 0;
            for (int i = 24; i + 6 <= end; i += 6 + BitConverter.ToUInt16(file, i + 4))
                if (Encoding.ASCII.GetString(file, i, 4) == "MAST") count++;
            return count;
        }

        public byte[] ToBytes() => _bytes.ToArray();

        public string WriteTo(string directory, string fileName)
        {
            string path = Path.Combine(directory, fileName);
            File.WriteAllBytes(path, ToBytes());
            return path;
        }

        void WriteRecord(string signature, int flags, uint formId, byte[] data)
        {
            _bytes.AddRange(Encoding.ASCII.GetBytes(signature));
            _bytes.AddRange(BitConverter.GetBytes(data.Length));
            _bytes.AddRange(BitConverter.GetBytes(flags));
            _bytes.AddRange(BitConverter.GetBytes(formId));
            _bytes.AddRange(new byte[8]);
            _bytes.AddRange(data);
        }
    }
}
