using System.Globalization;

namespace AssetConverter
{
    /// <summary>The depth of field fields of an IMGS record Mutagen cannot carry.</summary>
    internal sealed record ImageSpaceDofBlock(float Mode, float VignetteRadius, float VignetteStrength)
    {
        /// <summary>A record without a <c>DNAM</c> subrecord has no mode and no vignette.</summary>
        public static readonly ImageSpaceDofBlock None = new(0f, 0f, 0f);
    }

    /// <summary>
    /// Raw scan of the <c>DNAM</c> subrecord of IMGS records: float strength, distance, range, the mode, and, in the
    /// 24-byte form, the vignette radius and strength (the 16-byte form has no vignette). The mode is a
    /// float that is truncated (bit 0..1 front/back mode, bit 2 no sky, <c>mode &gt;&gt; 3</c> the blur radius index).
    /// Mutagen splits those four bytes into a short, a byte and a bool, losing the top byte, so the float is decoded here.
    /// </summary>
    internal static class ImageSpaceDofScan
    {
        public const int ModeOffset = 12;
        public const int VignetteOffset = 16;
        public const int WithoutVignetteSize = 16;
        public const int WithVignetteSize = 24;
        const string DofSignature = "DNAM";
        const string FailureSite = "IMGS DNAM depth of field";

        static readonly HashSet<string> RecordTypes = new(StringComparer.Ordinal) { "IMGS" };

        public static Dictionary<string, ImageSpaceDofBlock> Scan(string esmPath)
        {
            var result = new Dictionary<string, ImageSpaceDofBlock>(StringComparer.OrdinalIgnoreCase);
            try
            {
                RawEsmScanner.Scan(esmPath, "image space raw scan", RecordTypes, Decode, result);
            }
            catch (Exception ex) when (ex is IOException or InvalidDataException)
            {
                Console.WriteLine($"[WARN] image space raw scan failed, its data is left out: {ex.GetType().Name}: {ex.Message}");
            }
            return result;
        }

        /// <summary>Decodes one record's subrecords; null when it has no <c>DNAM</c> or an unusable one.</summary>
        internal static ImageSpaceDofBlock? Decode(string subject, byte[] recordData)
        {
            byte[]? dnam = null;
            foreach (var (signature, body) in RawEsmScanner.Subrecords(recordData))
                if (signature == DofSignature) dnam = body;
            if (dnam == null) return null;
            if (dnam.Length != WithoutVignetteSize && dnam.Length < WithVignetteSize)
            {
                ReadFailureTally.Shared.Record(FailureSite,
                    new InvalidDataException(string.Create(CultureInfo.InvariantCulture, $"DNAM has {dnam.Length} bytes, expected {WithoutVignetteSize} or at least {WithVignetteSize}")), subject);
                return null;
            }
            bool hasVignette = dnam.Length >= WithVignetteSize;
            return new ImageSpaceDofBlock(
                BitConverter.ToSingle(dnam, ModeOffset),
                hasVignette ? BitConverter.ToSingle(dnam, VignetteOffset) : 0f,
                hasVignette ? BitConverter.ToSingle(dnam, VignetteOffset + 4) : 0f);
        }
    }
}
