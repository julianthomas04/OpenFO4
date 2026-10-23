using System.Text;

namespace InstallerTests.Support
{
    /// <summary>Common header and base-class fields of BGSM and BGEM files
    /// (base class BGSMBase fields). Defaults describe a plain opaque material.</summary>
    internal sealed class MaterialBaseFields
    {
        public uint TileFlags { get; set; } = 3;
        public float UOffset { get; set; }
        public float VOffset { get; set; }
        public float UScale { get; set; } = 1f;
        public float VScale { get; set; } = 1f;
        public float Alpha { get; set; } = 1f;
        public byte BlendA { get; set; }
        public uint BlendB { get; set; }
        public uint BlendC { get; set; }
        public byte AlphaTestRef { get; set; } = 128;
        public bool AlphaTest { get; set; }
        public bool ZBufferWrite { get; set; } = true;
        public bool ZBufferTest { get; set; } = true;
        public bool ScreenSpaceReflections { get; set; }
        public bool WetnessControlSsr { get; set; }
        public bool Decal { get; set; }
        public bool TwoSided { get; set; }
        public bool DecalNoFade { get; set; }
        public bool NonOccluder { get; set; }
        public bool Refraction { get; set; }
        public bool RefractionFalloff { get; set; }
        public float RefractionPower { get; set; }
        public bool EnvironmentMapping { get; set; }
        public float EnvironmentMappingMaskScale { get; set; } = 1f;
        public bool DepthBias { get; set; }
        public bool GrayscaleToPalette { get; set; }
        public byte MaskWrites { get; set; } = 0x3F;
    }

    internal static class MaterialFileWriter
    {
        public static void WriteBase(BinaryWriter w, string magic, uint version, MaterialBaseFields f)
        {
            w.Write(Encoding.ASCII.GetBytes(magic));
            w.Write(version);
            w.Write(f.TileFlags);
            w.Write(f.UOffset);
            w.Write(f.VOffset);
            w.Write(f.UScale);
            w.Write(f.VScale);
            w.Write(f.Alpha);
            w.Write(f.BlendA);
            w.Write(f.BlendB);
            w.Write(f.BlendC);
            w.Write(f.AlphaTestRef);
            w.Write(f.AlphaTest);
            w.Write(f.ZBufferWrite);
            w.Write(f.ZBufferTest);
            w.Write(f.ScreenSpaceReflections);
            w.Write(f.WetnessControlSsr);
            w.Write(f.Decal);
            w.Write(f.TwoSided);
            w.Write(f.DecalNoFade);
            w.Write(f.NonOccluder);
            w.Write(f.Refraction);
            w.Write(f.RefractionFalloff);
            w.Write(f.RefractionPower);
            if (version < 10)
            {
                w.Write(f.EnvironmentMapping);
                w.Write(f.EnvironmentMappingMaskScale);
            }
            else
            {
                w.Write(f.DepthBias);
            }
            w.Write(f.GrayscaleToPalette);
            if (version >= 6)
                w.Write(f.MaskWrites);
        }

        /// <summary>Length-prefixed string; the length counts the trailing NUL that real files carry.</summary>
        public static void WriteString(BinaryWriter w, string value, bool nulTerminated = true)
        {
            w.Write((uint)(value.Length + (nulTerminated ? 1 : 0)));
            w.Write(Encoding.ASCII.GetBytes(value));
            if (nulTerminated)
                w.Write((byte)0);
        }

        public static void WriteColor(BinaryWriter w, float r, float g, float b)
        {
            w.Write(r);
            w.Write(g);
            w.Write(b);
        }
    }

    /// <summary>BGSM body (BGSM-specific fields); version gates follow the (v >= n) annotations
    /// and the community-documented layout the installer reads.</summary>
    internal sealed class BgsmFields
    {
        public MaterialBaseFields Base { get; set; } = new();
        public string Diffuse { get; set; } = "";
        public bool DiffuseAsBareZeroLength { get; set; }
        public string Normal { get; set; } = "";
        public string SmoothSpec { get; set; } = "";
        public string Greyscale { get; set; } = "";
        public string Envmap { get; set; } = "";
        public string Glow { get; set; } = "";
        public string InnerLayer { get; set; } = "";
        public string Wrinkles { get; set; } = "";
        public string Displacement { get; set; } = "";
        public string Specular { get; set; } = "";
        public string Lighting { get; set; } = "";
        public string Flow { get; set; } = "";
        public string DistanceField { get; set; } = "";
        public bool EnableEditorAlphaRef { get; set; }
        public bool Translucency { get; set; }
        public bool TranslucencyThick { get; set; }
        public bool TranslucencyMixed { get; set; }
        public (float R, float G, float B) TranslucencyColor { get; set; } = (0f, 0f, 0f);
        public float TranslucencyTransmissiveScale { get; set; } = 1.5f;
        public float TranslucencyTurbulence { get; set; } = 2.5f;
        public bool RimLighting { get; set; }
        public float RimPower { get; set; } = 2f;
        public float BacklightPower { get; set; }
        public bool SubsurfaceLighting { get; set; }
        public float SubsurfaceRolloff { get; set; } = 0.3f;
        public bool SpecularEnabled { get; set; }
        public (float R, float G, float B) SpecularColor { get; set; } = (1f, 1f, 1f);
        public float SpecularMult { get; set; } = 1f;
        public float Smoothness { get; set; } = 1f;
        public float FresnelPower { get; set; } = 5f;
        public float WetSpecScale { get; set; } = -1f;
        public float WetSpecPower { get; set; } = -1f;
        public float WetSpecMinVar { get; set; } = -1f;
        public float WetEnvMapScale { get; set; } = -1f;
        public float WetFresnelPower { get; set; } = -1f;
        public float WetMetalness { get; set; } = -1f;
        public bool Pbr { get; set; }
        public bool CustomPorosity { get; set; }
        public float Porosity { get; set; }
        public string RootMaterial { get; set; } = "";
        public bool Aniso { get; set; }
        public bool EmitEnabled { get; set; }
        public (float R, float G, float B) EmitColor { get; set; } = (1f, 1f, 1f);
        public float EmitMult { get; set; } = 1f;
        public bool ModelSpaceNormals { get; set; }
        public bool ExternalEmittance { get; set; }
        public float LumEmittance { get; set; }
        public bool UseAdaptiveEmissive { get; set; }
        public float AdaptiveOffset { get; set; }
        public float AdaptiveMin { get; set; }
        public float AdaptiveMax { get; set; }
        public bool BackLighting { get; set; }
        public bool ReceiveShadows { get; set; }
        public bool HideSecret { get; set; }
        public bool CastShadows { get; set; }
        public bool DissolveFade { get; set; }
        public bool AssumeShadowmask { get; set; }
        public bool Glowmap { get; set; }
        public bool EnvWindow { get; set; }
        public bool EnvEye { get; set; }
        public bool Hair { get; set; }
        public (float R, float G, float B) HairTint { get; set; } = (128f / 255f, 128f / 255f, 128f / 255f);
        public bool Tree { get; set; }
        public bool Facegen { get; set; }
        public bool SkinTint { get; set; }
        public bool Tessellate { get; set; }
        public float DisplacementBias { get; set; } = -0.5f;
        public float DisplacementScale { get; set; } = 10f;
        public float TessPnScale { get; set; } = 1f;
        public float TessBaseFactor { get; set; } = 1f;
        public float TessFadeDistance { get; set; }
        public float GrayscaleToPaletteScale { get; set; } = 1f;
        public bool SkewSpecularAlpha { get; set; }
        public bool Terrain { get; set; }
        public uint TerrainUnknown { get; set; }
        public float TerrainThresholdFalloff { get; set; }
        public float TerrainTilingDistance { get; set; }
        public float TerrainRotationAngle { get; set; }

        public byte[] Build(uint version)
        {
            using var stream = new MemoryStream();
            using var w = new BinaryWriter(stream);
            MaterialFileWriter.WriteBase(w, "BGSM", version, Base);
            if (DiffuseAsBareZeroLength)
                w.Write(0u);
            else
                MaterialFileWriter.WriteString(w, Diffuse);
            MaterialFileWriter.WriteString(w, Normal);
            MaterialFileWriter.WriteString(w, SmoothSpec);
            MaterialFileWriter.WriteString(w, Greyscale);
            if (version > 2)
            {
                MaterialFileWriter.WriteString(w, Glow);
                MaterialFileWriter.WriteString(w, Wrinkles);
                MaterialFileWriter.WriteString(w, Specular);
                MaterialFileWriter.WriteString(w, Lighting);
                MaterialFileWriter.WriteString(w, Flow);
                if (version >= 17)
                    MaterialFileWriter.WriteString(w, DistanceField);
            }
            else
            {
                MaterialFileWriter.WriteString(w, Envmap);
                MaterialFileWriter.WriteString(w, Glow);
                MaterialFileWriter.WriteString(w, InnerLayer);
                MaterialFileWriter.WriteString(w, Wrinkles);
                MaterialFileWriter.WriteString(w, Displacement);
            }
            w.Write(EnableEditorAlphaRef);
            if (version >= 8)
            {
                w.Write(Translucency);
                w.Write(TranslucencyThick);
                w.Write(TranslucencyMixed);
                MaterialFileWriter.WriteColor(w, TranslucencyColor.R, TranslucencyColor.G, TranslucencyColor.B);
                w.Write(TranslucencyTransmissiveScale);
                w.Write(TranslucencyTurbulence);
            }
            else
            {
                w.Write(RimLighting);
                w.Write(RimPower);
                w.Write(BacklightPower);
                w.Write(SubsurfaceLighting);
                w.Write(SubsurfaceRolloff);
            }
            w.Write(SpecularEnabled);
            MaterialFileWriter.WriteColor(w, SpecularColor.R, SpecularColor.G, SpecularColor.B);
            w.Write(SpecularMult);
            w.Write(Smoothness);
            w.Write(FresnelPower);
            w.Write(WetSpecScale);
            w.Write(WetSpecPower);
            w.Write(WetSpecMinVar);
            if (version < 10)
                w.Write(WetEnvMapScale);
            w.Write(WetFresnelPower);
            w.Write(WetMetalness);
            if (version > 2)
            {
                w.Write(Pbr);
                if (version >= 9)
                {
                    w.Write(CustomPorosity);
                    w.Write(Porosity);
                }
            }
            MaterialFileWriter.WriteString(w, RootMaterial);
            w.Write(Aniso);
            w.Write(EmitEnabled);
            if (EmitEnabled)
                MaterialFileWriter.WriteColor(w, EmitColor.R, EmitColor.G, EmitColor.B);
            w.Write(EmitMult);
            w.Write(ModelSpaceNormals);
            w.Write(ExternalEmittance);
            if (version >= 12)
                w.Write(LumEmittance);
            if (version >= 13)
            {
                w.Write(UseAdaptiveEmissive);
                w.Write(AdaptiveOffset);
                w.Write(AdaptiveMin);
                w.Write(AdaptiveMax);
            }
            if (version < 8)
                w.Write(BackLighting);
            w.Write(ReceiveShadows);
            w.Write(HideSecret);
            w.Write(CastShadows);
            w.Write(DissolveFade);
            w.Write(AssumeShadowmask);
            w.Write(Glowmap);
            if (version < 7)
            {
                w.Write(EnvWindow);
                w.Write(EnvEye);
            }
            w.Write(Hair);
            MaterialFileWriter.WriteColor(w, HairTint.R, HairTint.G, HairTint.B);
            w.Write(Tree);
            w.Write(Facegen);
            w.Write(SkinTint);
            w.Write(Tessellate);
            if (version < 3)
            {
                w.Write(DisplacementBias);
                w.Write(DisplacementScale);
                w.Write(TessPnScale);
                w.Write(TessBaseFactor);
                w.Write(TessFadeDistance);
            }
            w.Write(GrayscaleToPaletteScale);
            if (version >= 1)
                w.Write(SkewSpecularAlpha);
            if (version >= 3)
            {
                w.Write(Terrain);
                if (Terrain)
                {
                    if (version == 3)
                        w.Write(TerrainUnknown);
                    w.Write(TerrainThresholdFalloff);
                    w.Write(TerrainTilingDistance);
                    w.Write(TerrainRotationAngle);
                }
            }
            w.Flush();
            return stream.ToArray();
        }
    }

    /// <summary>BGEM body (BGEM-specific fields).</summary>
    internal sealed class BgemFields
    {
        public MaterialBaseFields Base { get; set; } = new();
        public string BaseTexture { get; set; } = "";
        public string Greyscale { get; set; } = "";
        public string Envmap { get; set; } = "";
        public string Normal { get; set; } = "";
        public string EnvmapMask { get; set; } = "";
        public string Specular { get; set; } = "";
        public string Lighting { get; set; } = "";
        public string Glow { get; set; } = "";
        public string GlassRoughnessScratch { get; set; } = "";
        public string GlassDirtOverlay { get; set; } = "";
        public bool GlassEnabled { get; set; }
        public (float R, float G, float B) GlassFresnelColor { get; set; } = (1f, 1f, 1f);
        public float GlassBlurScaleBase { get; set; } = 0.05f;
        public float GlassBlurScaleFactor { get; set; } = 1f;
        public float GlassRefractionScaleBase { get; set; } = 0.4f;
        public bool Blood { get; set; }
        public bool EffectLighting { get; set; }
        public bool Falloff { get; set; }
        public bool FalloffColor { get; set; }
        public bool GrayscaleToPaletteAlpha { get; set; }
        public bool Soft { get; set; }
        public (float R, float G, float B) BaseColor { get; set; } = (1f, 1f, 1f);
        public float BaseColorScale { get; set; } = 1f;
        public float FalloffStartAngle { get; set; }
        public float FalloffStopAngle { get; set; }
        public float FalloffStartOpacity { get; set; }
        public float FalloffStopOpacity { get; set; }
        public float LightingInfluence { get; set; } = 1f;
        public byte EnvmapMinLod { get; set; }
        public float SoftDepth { get; set; } = 100f;
        public (float R, float G, float B) EmitColor { get; set; } = (1f, 1f, 1f);
        public float AdaptiveOffset { get; set; }
        public float AdaptiveMin { get; set; }
        public float AdaptiveMax { get; set; }
        public bool Glowmap { get; set; }
        public bool EffectPbrSpecular { get; set; }

        public byte[] Build(uint version)
        {
            using var stream = new MemoryStream();
            using var w = new BinaryWriter(stream);
            MaterialFileWriter.WriteBase(w, "BGEM", version, Base);
            MaterialFileWriter.WriteString(w, BaseTexture);
            MaterialFileWriter.WriteString(w, Greyscale);
            MaterialFileWriter.WriteString(w, Envmap);
            MaterialFileWriter.WriteString(w, Normal);
            MaterialFileWriter.WriteString(w, EnvmapMask);
            if (version >= 11)
            {
                MaterialFileWriter.WriteString(w, Specular);
                MaterialFileWriter.WriteString(w, Lighting);
                MaterialFileWriter.WriteString(w, Glow);
            }
            if (version >= 21)
            {
                MaterialFileWriter.WriteString(w, GlassRoughnessScratch);
                MaterialFileWriter.WriteString(w, GlassDirtOverlay);
                w.Write(GlassEnabled);
                if (GlassEnabled)
                {
                    MaterialFileWriter.WriteColor(w, GlassFresnelColor.R, GlassFresnelColor.G, GlassFresnelColor.B);
                    w.Write(GlassBlurScaleBase);
                    if (version >= 22)
                        w.Write(GlassBlurScaleFactor);
                    w.Write(GlassRefractionScaleBase);
                }
            }
            if (version >= 10)
            {
                w.Write(Base.EnvironmentMapping);
                w.Write(Base.EnvironmentMappingMaskScale);
            }
            w.Write(Blood);
            w.Write(EffectLighting);
            w.Write(Falloff);
            w.Write(FalloffColor);
            w.Write(GrayscaleToPaletteAlpha);
            w.Write(Soft);
            // The format spec lists base_color as Color4, but the installer's reader consumes three floats.
            MaterialFileWriter.WriteColor(w, BaseColor.R, BaseColor.G, BaseColor.B);
            w.Write(BaseColorScale);
            w.Write(FalloffStartAngle);
            w.Write(FalloffStopAngle);
            w.Write(FalloffStartOpacity);
            w.Write(FalloffStopOpacity);
            w.Write(LightingInfluence);
            w.Write(EnvmapMinLod);
            w.Write(SoftDepth);
            if (version >= 11)
                MaterialFileWriter.WriteColor(w, EmitColor.R, EmitColor.G, EmitColor.B);
            if (version >= 15)
            {
                w.Write(AdaptiveOffset);
                w.Write(AdaptiveMin);
                w.Write(AdaptiveMax);
            }
            if (version >= 16)
                w.Write(Glowmap);
            if (version >= 20)
                w.Write(EffectPbrSpecular);
            w.Flush();
            return stream.ToArray();
        }
    }
}
