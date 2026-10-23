using System.Text;

namespace AssetConverter
{
    /// <summary>
    /// The key Fallout 4 uses to join Havok shapes to MATT records: CRC-32 (reflected polynomial 0xEDB88320,
    /// initial value 0, no final inversion) of the lower-cased MATT material name (MNAM). The engine
    /// uses this hash as the key to look up material records.
    /// The shapes carry the same value in the low dword of their user data.
    /// </summary>
    public static class BgsMaterialHash
    {
        static readonly uint[] Table = BuildTable();

        public static uint Compute(string materialName)
        {
            uint crc = 0;
            foreach (byte b in Encoding.Latin1.GetBytes(materialName.ToLowerInvariant()))
                crc = Table[(crc ^ b) & 0xFF] ^ (crc >> 8);
            return crc;
        }

        static uint[] BuildTable()
        {
            var table = new uint[256];
            for (uint i = 0; i < table.Length; i++)
            {
                uint value = i;
                for (int bit = 0; bit < 8; bit++)
                    value = (value & 1) != 0 ? (value >> 1) ^ 0xEDB88320u : value >> 1;
                table[i] = value;
            }
            return table;
        }
    }
}
