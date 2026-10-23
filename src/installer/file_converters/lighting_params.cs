using System.Reflection;
using NiflySharp.Blocks;
using NiflySharp.Structs;

namespace AssetConverter
{
    /// <summary>Specular and emissive shading inputs of one material. Colours are
    /// packed 0xRRGGBB sRGB bytes; texture paths are raw as authored (empty = none).
    /// The glow texture is only set when the material enables its glow map.</summary>
    public sealed record LightingParams(
        bool SpecularEnabled,
        int SpecularColor,
        float SpecularMult,
        float Smoothness,
        float FresnelPower,
        bool EmissiveEnabled,
        int EmissiveColor,
        float EmissiveMult,
        string SpecularTexture,
        string GlowTexture)
    {
        private const int SpecularSlot = 7;
        private const int GlowSlot = 2;

        public static readonly LightingParams None = new(
            SpecularEnabled: false, SpecularColor: 0xFFFFFF, SpecularMult: 1f, Smoothness: 1f, FresnelPower: 5f,
            EmissiveEnabled: false, EmissiveColor: 0xFFFFFF, EmissiveMult: 1f,
            SpecularTexture: "", GlowTexture: "");

        private static readonly FieldInfo SpecularColorField = Field("_specularColor");
        private static readonly FieldInfo SpecularStrengthField = Field("_specularStrength");
        private static readonly FieldInfo SmoothnessField = Field("_smoothness");
        private static readonly FieldInfo FresnelPowerField = Field("_fresnelPower");
        private static readonly FieldInfo EmissiveMultipleField = Field("_emissiveMultiple");

        public static LightingParams FromBgsm(BGSM bgsm) => new(
            SpecularEnabled: bgsm.SpecularEnabled,
            SpecularColor: (int)(bgsm.SpecularColor & 0xFFFFFFu),
            SpecularMult: bgsm.SpecularMult,
            Smoothness: bgsm.Smoothness,
            FresnelPower: bgsm.FresnelPower,
            EmissiveEnabled: bgsm.EmitEnabled,
            EmissiveColor: (int)(bgsm.EmittanceColor & 0xFFFFFFu),
            EmissiveMult: bgsm.EmittanceMult,
            SpecularTexture: bgsm.SmoothSpecTexture ?? "",
            GlowTexture: bgsm.Glowmap ? bgsm.GlowTexture ?? "" : "");

        public static LightingParams FromInline(BSLightingShaderProperty shader, BSShaderTextureSet? textureSet) => new(
            SpecularEnabled: shader.HasSpecular,
            SpecularColor: Pack((Color3)SpecularColorField.GetValue(shader)!),
            SpecularMult: (float)SpecularStrengthField.GetValue(shader)!,
            Smoothness: (float)SmoothnessField.GetValue(shader)!,
            FresnelPower: (float)FresnelPowerField.GetValue(shader)!,
            EmissiveEnabled: shader.Emissive,
            EmissiveColor: Pack(shader.EmissiveColor),
            EmissiveMult: (float)EmissiveMultipleField.GetValue(shader)!,
            SpecularTexture: Slot(textureSet, SpecularSlot),
            GlowTexture: shader.HasGlowmap ? Slot(textureSet, GlowSlot) : "");

        public LightingParams WithTexturePaths(Func<string, string> clean) => this with
        {
            SpecularTexture = clean(SpecularTexture),
            GlowTexture = clean(GlowTexture),
        };

        private static FieldInfo Field(string name) =>
            typeof(BSLightingShaderProperty).GetField(name, BindingFlags.NonPublic | BindingFlags.Instance)
            ?? throw new MissingFieldException(nameof(BSLightingShaderProperty), name);

        private static string Slot(BSShaderTextureSet? textureSet, int index) =>
            textureSet?.Textures != null && textureSet.Textures.Count > index
                ? textureSet.Textures[index]?.Content ?? ""
                : "";

        private static int Pack(Color3 c) => Pack(c.R, c.G, c.B);
        private static int Pack(Color4 c) => Pack(c.R, c.G, c.B);

        private static int Pack(float r, float g, float b) => (Byte(r) << 16) | (Byte(g) << 8) | Byte(b);

        private static int Byte(float channel) => (int)MathF.Round(Math.Clamp(channel, 0f, 1f) * 255f);
    }
}
