
namespace AssetConverter
{
    public class BGEM : BaseMaterialFile
    {
        public const uint Signature = 0x4D454742u;

        #region Fields
        protected override void SetDefaults()
        {
            base.SetDefaults();

            BaseTexture = "";
            GrayscaleTexture = "";
            EnvmapTexture = "";
            NormalTexture = "";
            EnvmapMaskTexture = "";
            SpecularTexture = "";
            LightingTexture = "";
            GlowTexture = "";

            BaseColor = 0xFFFFFFFFu;
            BaseColorScale = 1.0f;
            FalloffStartAngle = 1.0f;
            FalloffStopAngle = 1.0f;
            LightingInfluence = 1.0f;
            SoftDepth = 100.0f;
            EmittanceColor = 0xFFFFFFFFu;

            GlassRoughnessScratch = "";
            GlassDirtOverlay = "";

            GlassFresnelColor = 0xFFFFFFFFu;
            GlassRefractionScaleBase = 0.05f;
            GlassBlurScaleBase = 0.4f;
            GlassBlurScaleFactor = 1.0f;
        }
        #endregion

        public BGEM()
            : base(Signature)
        {
        }

        #region Properties
        public string? BaseTexture { get; set; }

        public string? GrayscaleTexture { get; set; }

        public string? EnvmapTexture { get; set; }

        public string? NormalTexture { get; set; }

        public string? EnvmapMaskTexture { get; set; }

        public string? SpecularTexture { get; set; }

        public string? LightingTexture { get; set; }

        public string? GlowTexture { get; set; }

        public bool BloodEnabled { get; set; }

        public bool EffectLightingEnabled { get; set; }

        public bool FalloffEnabled { get; set; }

        public bool FalloffColorEnabled { get; set; }

        public bool GrayscaleToPaletteAlpha { get; set; }

        public bool SoftEnabled { get; set; }

        public uint BaseColor { get; set; }

        public float BaseColorScale { get; set; }

        public float FalloffStartAngle { get; set; }

        public float FalloffStopAngle { get; set; }

        public float FalloffStartOpacity { get; set; }

        public float FalloffStopOpacity { get; set; }

        public float LightingInfluence { get; set; }

        public byte EnvmapMinLOD { get; set; }

        public float SoftDepth { get; set; }

        public uint EmittanceColor { get; set; }

        public float AdaptativeEmissive_ExposureOffset { get; set; }

        public float AdaptativeEmissive_FinalExposureMin { get; set; }

        public float AdaptativeEmissive_FinalExposureMax { get; set; }

        public bool Glowmap { get; set; }

        public bool EffectPbrSpecular { get; set; }

        public string? GlassRoughnessScratch { get; set; }

        public string? GlassDirtOverlay { get; set; }

        public bool GlassEnabled { get; set; }

        public uint GlassFresnelColor { get; set; }

        public float GlassRefractionScaleBase { get; set; }

        public float GlassBlurScaleBase { get; set; }

        public float GlassBlurScaleFactor { get; set; }
        #endregion

        public override void Deserialize(BinaryReader input)
        {
            base.Deserialize(input);

            BaseTexture = ReadString(input);
            GrayscaleTexture = ReadString(input);
            EnvmapTexture = ReadString(input);
            NormalTexture = ReadString(input);
            EnvmapMaskTexture = ReadString(input);

            if (Version >= 11)
            {
                SpecularTexture = ReadString(input);
                LightingTexture = ReadString(input);
                GlowTexture = ReadString(input);
            }

            if (Version >= 21)
            {
                GlassRoughnessScratch = ReadString(input);
                GlassDirtOverlay = ReadString(input);
                GlassEnabled = input.ReadBoolean();

                if (GlassEnabled)
                {
                    GlassFresnelColor = Color.Read(input).ToUInt32(); // 1.0, 1.0, 1.0

                    // FIXME: Order might be wrong
                    GlassBlurScaleBase = input.ReadSingle(); // Occurring values: 0.05, 0.005, 0.01, -0.09, 0.0

                    if (Version >= 22)
                        GlassBlurScaleFactor = input.ReadSingle(); // Occurring values: 1.0, 0.0

                    GlassRefractionScaleBase = input.ReadSingle(); // Occurring values: 0.4, 0.1, 0.01, 0.0
                }
            }

            if (Version >= 10)
            {
                EnvironmentMapping = input.ReadBoolean();
                EnvironmentMappingMaskScale = input.ReadSingle();
            }

            BloodEnabled = input.ReadBoolean();
            EffectLightingEnabled = input.ReadBoolean();
            FalloffEnabled = input.ReadBoolean();
            FalloffColorEnabled = input.ReadBoolean();
            GrayscaleToPaletteAlpha = input.ReadBoolean();
            SoftEnabled = input.ReadBoolean();

            BaseColor = Color.Read(input).ToRoundedUInt32();
            BaseColorScale = input.ReadSingle();

            FalloffStartAngle = input.ReadSingle();
            FalloffStopAngle = input.ReadSingle();
            FalloffStartOpacity = input.ReadSingle();
            FalloffStopOpacity = input.ReadSingle();

            LightingInfluence = input.ReadSingle();
            EnvmapMinLOD = input.ReadByte();
            SoftDepth = input.ReadSingle();

            if (Version >= 11)
            {
                EmittanceColor = Color.Read(input).ToUInt32();
            }

            if (Version >= 15)
            {
                AdaptativeEmissive_ExposureOffset = input.ReadSingle();
                AdaptativeEmissive_FinalExposureMin = input.ReadSingle();
                AdaptativeEmissive_FinalExposureMax = input.ReadSingle();
            }

            if (Version >= 16)
            {
                Glowmap = input.ReadBoolean();
            }

            if (Version >= 20)
            {
                EffectPbrSpecular = input.ReadBoolean();
            }
        }

        public override void Serialize(BinaryWriter output)
        {
            base.Serialize(output);

            WriteString(output, BaseTexture);
            WriteString(output, GrayscaleTexture);
            WriteString(output, EnvmapTexture);
            WriteString(output, NormalTexture);
            WriteString(output, EnvmapMaskTexture);

            if (Version >= 11)
            {
                WriteString(output, SpecularTexture);
                WriteString(output, LightingTexture);
                WriteString(output, GlowTexture);
            }

            if (Version >= 21)
            {
                WriteString(output, GlassRoughnessScratch);
                WriteString(output, GlassDirtOverlay);
                output.Write(GlassEnabled);

                if (GlassEnabled)
                {
                    Color.FromUInt32(GlassFresnelColor).Write(output);

                    // FIXME: Order might be wrong
                    output.Write(GlassBlurScaleBase);

                    if (Version >= 22)
                        output.Write(GlassBlurScaleFactor);

                    output.Write(GlassRefractionScaleBase);
                }
            }

            if (Version >= 10)
            {
                output.Write(EnvironmentMapping);
                output.Write(EnvironmentMappingMaskScale);
            }

            output.Write(BloodEnabled);
            output.Write(EffectLightingEnabled);
            output.Write(FalloffEnabled);
            output.Write(FalloffColorEnabled);
            output.Write(GrayscaleToPaletteAlpha);
            output.Write(SoftEnabled);

            Color.FromUInt32(BaseColor).Write(output);
            output.Write(BaseColorScale);

            output.Write(FalloffStartAngle);
            output.Write(FalloffStopAngle);
            output.Write(FalloffStartOpacity);
            output.Write(FalloffStopOpacity);

            output.Write(LightingInfluence);
            output.Write(EnvmapMinLOD);
            output.Write(SoftDepth);

            if (Version >= 11)
            {
                Color.FromUInt32(EmittanceColor).Write(output);
            }

            if (Version >= 15)
            {
                output.Write(AdaptativeEmissive_ExposureOffset);
                output.Write(AdaptativeEmissive_FinalExposureMin);
                output.Write(AdaptativeEmissive_FinalExposureMax);
            }

            if (Version >= 16)
            {
                output.Write(Glowmap);
            }

            if (Version >= 20)
            {
                output.Write(EffectPbrSpecular);
            }
        }
    }
}
