using System.Globalization;

namespace AssetConverter
{
    public abstract class BaseMaterialFile
    {
        private readonly uint _Signature;

        #region Fields
        protected virtual void SetDefaults()
        {
            Version = 2;
            TileU = true;
            TileV = true;
            UScale = 1.0f;
            VScale = 1.0f;
            Alpha = 1.0f;
            AlphaTestRef = 128;
            ZBufferWrite = true;
            ZBufferTest = true;
            EnvironmentMappingMaskScale = 1.0f;
            MaskWrites = MaskWriteChannels.ALBEDO | MaskWriteChannels.NORMAL | MaskWriteChannels.SPECULAR | MaskWriteChannels.AMBIENT_OCCLUSION | MaskWriteChannels.EMISSIVE | MaskWriteChannels.GLOSS;
        }
        #endregion

        protected BaseMaterialFile(uint signature)
        {
            _Signature = signature;
            SetDefaults();
        }

        #region Properties
        public uint Version { get; set; }

        public bool TileU { get; set; }

        public bool TileV { get; set; }

        public float UOffset { get; set; }

        public float VOffset { get; set; }

        public float UScale { get; set; }

        public float VScale { get; set; }

        public float Alpha { get; set; }

        public AlphaBlendModeType AlphaBlendMode { get; set; }

        public byte AlphaTestRef { get; set; }

        public bool AlphaTest { get; set; }

        public bool ZBufferWrite { get; set; }

        public bool ZBufferTest { get; set; }

        public bool ScreenSpaceReflections { get; set; }

        public bool WetnessControlScreenSpaceReflections { get; set; }

        public bool Decal { get; set; }

        public bool TwoSided { get; set; }

        public bool DecalNoFade { get; set; }

        public bool NonOccluder { get; set; }

        public bool Refraction { get; set; }

        public bool RefractionFalloff { get; set; }

        public float RefractionPower { get; set; }

        public bool EnvironmentMapping { get; set; }

        public float EnvironmentMappingMaskScale { get; set; }

        public bool DepthBias { get; set; }

        public bool GrayscaleToPaletteColor { get; set; }

        public MaskWriteChannels MaskWrites { get; set; }
        #endregion

        public virtual void Deserialize(BinaryReader input)
        {
            var magic = input.ReadUInt32();
            if (magic != _Signature)
            {
                throw new FormatException();
            }

            Version = input.ReadUInt32();

            var tileFlags = input.ReadUInt32();
            TileU = (tileFlags & 2) != 0;
            TileV = (tileFlags & 1) != 0;
            UOffset = input.ReadSingle();
            VOffset = input.ReadSingle();
            UScale = input.ReadSingle();
            VScale = input.ReadSingle();

            Alpha = input.ReadSingle();
            var alphaBlendMode0 = input.ReadByte();
            var alphaBlendMode1 = input.ReadUInt32();
            var alphaBlendMode2 = input.ReadUInt32();
            AlphaBlendMode = ConvertAlphaBlendMode(alphaBlendMode0, alphaBlendMode1, alphaBlendMode2);
            AlphaTestRef = input.ReadByte();
            AlphaTest = input.ReadBoolean();

            ZBufferWrite = input.ReadBoolean();
            ZBufferTest = input.ReadBoolean();
            ScreenSpaceReflections = input.ReadBoolean();
            WetnessControlScreenSpaceReflections = input.ReadBoolean();
            Decal = input.ReadBoolean();
            TwoSided = input.ReadBoolean();
            DecalNoFade = input.ReadBoolean();
            NonOccluder = input.ReadBoolean();

            Refraction = input.ReadBoolean();
            RefractionFalloff = input.ReadBoolean();
            RefractionPower = input.ReadSingle();

            if (Version < 10)
            {
                EnvironmentMapping = input.ReadBoolean();
                EnvironmentMappingMaskScale = input.ReadSingle();
            }
            else
            {
                DepthBias = input.ReadBoolean();
            }

            GrayscaleToPaletteColor = input.ReadBoolean();

            if (Version >= 6)
            {
                MaskWrites = (MaskWriteChannels)input.ReadByte();
            }
        }

        public virtual void Serialize(BinaryWriter output)
        {
            output.Write(_Signature);
            output.Write(Version);

            uint tileFlags = 0;
            if (TileU) tileFlags += 2;
            if (TileV) tileFlags += 1;
            output.Write(tileFlags);

            output.Write(UOffset);
            output.Write(VOffset);
            output.Write(UScale);
            output.Write(VScale);

            output.Write(Alpha);

            byte alphaBlendMode0 = 0;
            uint alphaBlendMode1 = 0;
            uint alphaBlendMode2 = 0;
            ConvertAlphaBlendMode(AlphaBlendMode, ref alphaBlendMode0, ref alphaBlendMode1, ref alphaBlendMode2);
            output.Write(alphaBlendMode0);
            output.Write(alphaBlendMode1);
            output.Write(alphaBlendMode2);

            output.Write(AlphaTestRef);
            output.Write(AlphaTest);

            output.Write(ZBufferWrite);
            output.Write(ZBufferTest);
            output.Write(ScreenSpaceReflections);
            output.Write(WetnessControlScreenSpaceReflections);
            output.Write(Decal);
            output.Write(TwoSided);
            output.Write(DecalNoFade);
            output.Write(NonOccluder);

            output.Write(Refraction);
            output.Write(RefractionFalloff);
            output.Write(RefractionPower);

            if (Version < 10)
            {
                output.Write(EnvironmentMapping);
                output.Write(EnvironmentMappingMaskScale);
            }
            else
            {
                output.Write(DepthBias);
            }

            output.Write(GrayscaleToPaletteColor);

            if (Version >= 6)
            {
                output.Write((byte)MaskWrites);
            }
        }

        /// <summary>Reads the material from <paramref name="file"/>, leaving the stream open.</summary>
        /// <exception cref="InvalidDataException">The content is not a well-formed material of this type.</exception>
        /// <exception cref="EndOfStreamException">The stream ends before the material does.</exception>
        public void Open(Stream file)
        {
            using BinaryReader reader = new(file, System.Text.Encoding.UTF8, leaveOpen: true);
            try
            {
                Deserialize(reader);
            }
            catch (Exception e) when (e is FormatException or NotSupportedException or ArgumentException)
            {
                throw new InvalidDataException($"Malformed {GetType().Name}: {e.Message}", e);
            }
        }

        /// <summary>Whether <paramref name="e"/> is a way a single material file can fail to load.</summary>
        public static bool IsReadFailure(Exception e) =>
            e is IOException or UnauthorizedAccessException or InvalidDataException;

        // Strings are length-prefixed byte runs (texture paths); Latin-1 keeps every byte as one char.
        protected static string ReadString(BinaryReader input)
        {
            uint length = input.ReadUInt32();
            long remaining = input.BaseStream.Length - input.BaseStream.Position;
            if (length > remaining)
                throw new InvalidDataException($"String length {length} exceeds the {remaining} bytes left in the stream");

            string str = System.Text.Encoding.Latin1.GetString(input.ReadBytes((int)length));

            int index = str.LastIndexOf('\0');
            if (index >= 0)
                str = str.Remove(index, 1);

            return str;
        }

        protected static void WriteString(BinaryWriter output, string? str)
        {
            if (str != null)
            {
                int length = str.Length + 1;
                output.Write(length);
                output.Write((str + "\0").ToCharArray());
            }
            else
            {
                int length = 1;
                output.Write(length);
                output.Write("\0".ToCharArray());
            }
        }

        public enum AlphaBlendModeType
        {
            Unknown = 0,
            None,
            Standard,
            Additive,
            Multiplicative,
        }

        private static AlphaBlendModeType ConvertAlphaBlendMode(byte a, uint b, uint c)
        {
            if (a == 0 && b == 6 && c == 7)
            {
                return AlphaBlendModeType.Unknown;
            }
            else if (a == 0 && b == 0 && c == 0)
            {
                return AlphaBlendModeType.None;
            }
            else if (a == 1 && b == 6 && c == 7)
            {
                return AlphaBlendModeType.Standard;
            }
            else if (a == 1 && b == 6 && c == 0)
            {
                return AlphaBlendModeType.Additive;
            }
            else if (a == 1 && b == 4 && c == 1)
            {
                return AlphaBlendModeType.Multiplicative;
            }

            throw new NotSupportedException();
        }

        private static void ConvertAlphaBlendMode(AlphaBlendModeType type, ref byte a, ref uint b, ref uint c)
        {
            if (type == AlphaBlendModeType.Unknown)
            {
                a = 0;
                b = 6;
                c = 7;
            }
            else if (type == AlphaBlendModeType.None)
            {
                a = 0;
                b = 0;
                c = 0;
            }
            else if (type == AlphaBlendModeType.Standard)
            {
                a = 1;
                b = 6;
                c = 7;
            }
            else if (type == AlphaBlendModeType.Additive)
            {
                a = 1;
                b = 6;
                c = 0;
            }
            else if (type == AlphaBlendModeType.Multiplicative)
            {
                a = 1;
                b = 4;
                c = 1;
            }
            else
                throw new NotSupportedException();
        }

        protected readonly struct Color(float r, float g, float b)
        {
            public float R { get; } = r;
            public float G { get; } = g;
            public float B { get; } = b;

            public uint ToUInt32()
            {
                uint value = 0;
                value |= (byte)(R * 255);
                value <<= 8;
                value |= (byte)(G * 255);
                value <<= 8;
                value |= (byte)(B * 255);
                return value;
            }

            public uint ToRoundedUInt32()
            {
                static uint Channel(float c) => (uint)MathF.Round(Math.Clamp(float.IsNaN(c) ? 0f : c, 0f, 1f) * 255f);
                return (Channel(R) << 16) | (Channel(G) << 8) | Channel(B);
            }

            public static Color FromUInt32(uint value)
            {
                const float multiplier = 1.0f / 255;
                var b = (byte)(value & 0xFF);
                value >>= 8;
                var g = (byte)(value & 0xFF);
                value >>= 8;
                var r = (byte)(value & 0xFF);
                return new Color(r * multiplier, g * multiplier, b * multiplier);
            }

            public static Color Read(BinaryReader input)
            {
                var r = input.ReadSingle();
                var g = input.ReadSingle();
                var b = input.ReadSingle();
                return new Color(r, g, b);
            }

            public void Write(BinaryWriter output)
            {
                output.Write(R);
                output.Write(G);
                output.Write(B);
            }
        }

        public enum MaskWriteChannels
        {
            ALBEDO = 1,
            NORMAL = 2,
            SPECULAR = 4,
            AMBIENT_OCCLUSION = 8,
            EMISSIVE = 16,
            GLOSS = 32
        }
    }
}
