namespace AssetConverter
{
    /// <summary>Angle-falloff of an effect material. StartAngle/StopAngle are the facing-ratio
    /// (dot of normal and view direction) thresholds exactly as authored, in [-1, 1]; they are
    /// not degrees.</summary>
    public sealed record FalloffParams(
        float StartAngle,
        float StopAngle,
        float StartOpacity,
        float StopOpacity,
        bool ScalesColor);

    /// <summary>The effect-shader inputs the object-shader effect path shades with (name tokens bm bc bs fo li pal), derived from the
    /// same parsed data as the effect extras. Texture paths are raw as authored (empty = none); the BaseColor is 0xRRGGBB sRGB bytes.</summary>
    /// <param name="BlendMode">"alpha" | "add" | "mul".</param>
    /// <param name="LightingInfluence">null when effect lighting is off (unlit).</param>
    public sealed record EffectParams(
        string BlendMode,
        int BaseColor,
        float BaseColorScale,
        FalloffParams? Falloff,
        float? LightingInfluence,
        string PaletteTexture,
        bool PaletteColor,
        bool PaletteAlpha)
    {
        public const string BlendAlpha = "alpha";
        public const string BlendAdd = "add";
        public const string BlendMul = "mul";

        /// <summary>Derived from the same parsed material the effect extras are built from.</summary>
        internal static EffectParams FromMaterial(EffectMaterialParams material) => new(
            BlendMode: BlendModeOf(EffectBlendModes.FromMaterial(material.Blend)),
            BaseColor: Pack(material.BaseColor),
            BaseColorScale: material.BaseColorScale,
            Falloff: material.FalloffEnabled
                ? new FalloffParams(material.FalloffStartAngle, material.FalloffStopAngle,
                    material.FalloffStartOpacity, material.FalloffStopOpacity, material.FalloffColorEnabled)
                : null,
            LightingInfluence: material.EffectLightingEnabled ? ClampInfluence(material.LightingInfluence) : null,
            PaletteTexture: material.GreyscaleTexture,
            PaletteColor: material.GreyscaleToPaletteColor,
            PaletteAlpha: material.GreyscaleToPaletteAlpha);

        /// <summary>Derived from the inline shader block (shapes without a BGEM) and the shape's NiAlphaProperty.</summary>
        internal static EffectParams FromShader(EffectShaderData shader, EffectAlphaState? alpha) => new(
            BlendMode: BlendModeOf(alpha?.Mode ?? EffectBlend.None),
            BaseColor: Pack(shader.BaseColor),
            BaseColorScale: shader.BaseColorScale,
            Falloff: EffectShaderFlags.Has(shader.Flags1, EffectShaderFlags.Falloff)
                ? new FalloffParams(shader.FalloffStartAngle, shader.FalloffStopAngle,
                    shader.FalloffStartOpacity, shader.FalloffStopOpacity,
                    EffectShaderFlags.Has(shader.Flags1, EffectShaderFlags.ColorFalloff))
                : null,
            LightingInfluence: EffectShaderFlags.Has(shader.Flags2, EffectShaderFlags.EffectLighting)
                ? shader.LightingInfluence / LightingInfluenceByteMax
                : null,
            PaletteTexture: shader.GreyscaleTexture,
            PaletteColor: EffectShaderFlags.Has(shader.Flags1, EffectShaderFlags.PaletteColor),
            PaletteAlpha: EffectShaderFlags.Has(shader.Flags1, EffectShaderFlags.PaletteAlpha));
        public EffectParams WithTexturePaths(Func<string, string> clean) => this with
        {
            PaletteTexture = clean(PaletteTexture),
        };

        public bool HasPalette => PaletteTexture.Length > 0 && (PaletteColor || PaletteAlpha);

        private const float LightingInfluenceByteMax = 255f;

        internal static float ClampInfluence(float influence) => Math.Clamp(influence, 0f, 1f);

        private static string BlendModeOf(EffectBlend blend) => blend switch
        {
            EffectBlend.Add => BlendAdd,
            EffectBlend.Mult => BlendMul,
            _ => BlendAlpha,
        };

        private static int Pack(System.Numerics.Vector4 color) => (Channel(color.X) << 16) | (Channel(color.Y) << 8) | Channel(color.Z);

        private static int Channel(float value) => (int)MathF.Round(Math.Clamp(value, 0f, 1f) * 255f);
    }

    /// <summary>Environment-map inputs; texture paths raw as authored, MaskTexture empty when none.</summary>
    public sealed record EnvironmentParams(string CubeTexture, string MaskTexture, float MaskScale)
    {
        /// <summary>Null when mapping is off or the cube texture is not in the texture index.
        /// <paramref name="cubeMissing"/> is true for the second case so the caller can count it.
        /// A missing mask only drops the mask.</summary>
        public static EnvironmentParams? Resolve(bool enabled, string? cube, string? mask, float maskScale,
            Func<string, bool> textureExists, out bool cubeMissing)
        {
            cubeMissing = false;
            if (!enabled || string.IsNullOrEmpty(cube)) return null;
            if (!textureExists(MaterialCensus.NormalizeTexPath(cube)))
            {
                cubeMissing = true;
                return null;
            }
            bool maskExists = !string.IsNullOrEmpty(mask) && textureExists(MaterialCensus.NormalizeTexPath(mask));
            return new EnvironmentParams(cube, maskExists ? mask! : "", maskScale);
        }

        public EnvironmentParams WithTexturePaths(Func<string, string> clean) => this with
        {
            CubeTexture = clean(CubeTexture),
            MaskTexture = clean(MaskTexture),
        };
    }
}
