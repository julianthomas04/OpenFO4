using System.Globalization;

namespace AssetConverter
{
    /// <param name="Diffuse">Deployed diffuse path.</param>
    /// <param name="Normal">Deployed normal path.</param>
    /// <param name="AlphaToken">"" | "blend" | "blend|0.50" | "test:{0-255}".</param>
    /// <param name="Lighting">Texture paths already in deployed form.</param>
    /// <param name="SourceKey">Census key of the material file, null for inline-texture shapes.</param>
    /// <param name="Water">Shader fields of a BSWaterShaderProperty shape (token <c>water:</c>), null otherwise.</param>
    /// <param name="Refraction">Heat-haze shape routed to the refraction stage (token <c>refr</c>).</param>
    /// <param name="ExtrasKey">xxh32 of the shape's canonical material extras (token <c>eff:XXXXXXXX</c>), so differing parameters never share a material name.</param>
    /// <param name="EffectFields">Effect-shader fields (tokens <c>bm bc bs fo li pal</c>), texture paths in deployed form; null for non-effect shapes.</param>
    /// <param name="Env">Environment-map fields (tokens <c>env envk envs</c>), texture paths in deployed form; null when absent.</param>
    public sealed record MaterialNameParts(
        string Diffuse,
        string Normal,
        string AlphaToken,
        bool Effect,
        bool Decal,
        LightingParams Lighting,
        bool TwoSided,
        string? SourceKey,
        WaterShaderParams? Water = null,
        bool Refraction = false,
        uint? ExtrasKey = null,
        EffectParams? EffectFields = null,
        EnvironmentParams? Env = null);

    /// <summary>GLB material-name grammar, single producer. Segments in order:
    /// diffuse|normal[|alpha][|fx][|refr][|eff:XXXXXXXX][|decal][|water:WF:F1:F2:UOU,UOV:USU,USV][|bm:M][|bc:RRGGBB][|bs:F][|fo:SA,EA,SO,EO,C][|li:F][|pal:FLAGS:PATH][|env:PATH][|envk:PATH][|envs:F][|spec:P][|glow:P][|sc:RRGGBB][|sm:F][|sg:F][|fp:F][|em:RRGGBB][|emm:F][|ts][|src:KEY].
    /// The water token carries the raw BSWaterShaderProperty fields: WF decimal water flags, F1/F2 8-digit hex shader flag words, UO/US the UV offset and scale (round-trip floats).
    /// Specular tokens appear only when specular is enabled, em/emm only when
    /// emissive is enabled with a non-black colour and non-zero multiplier, glow only when a glow map is set.</summary>
    public static class MaterialNameCodec
    {
        public static string Encode(MaterialNameParts parts)
        {
            var segments = new List<string> { parts.Diffuse, parts.Normal };
            if (parts.AlphaToken.Length > 0) segments.Add(parts.AlphaToken);
            if (parts.Effect) segments.Add("fx");
            if (parts.Refraction) segments.Add("refr");
            if (parts.ExtrasKey is { } extrasKey) segments.Add($"eff:{extrasKey.ToString("x8", CultureInfo.InvariantCulture)}");
            if (parts.Decal) segments.Add("decal");
            if (parts.Water != null) segments.Add(parts.Water.ToToken());
            if (parts.EffectFields != null) AddEffectTokens(segments, parts.EffectFields);
            if (parts.Env != null) AddEnvironmentTokens(segments, parts.Env);

            var light = parts.Lighting;
            if (light.SpecularEnabled)
            {
                if (light.SpecularTexture.Length > 0) segments.Add($"spec:{light.SpecularTexture}");
                if (light.GlowTexture.Length > 0) segments.Add($"glow:{light.GlowTexture}");
                segments.Add($"sc:{Hex(light.SpecularColor)}");
                segments.Add($"sm:{Float(light.SpecularMult)}");
                segments.Add($"sg:{Float(light.Smoothness)}");
                segments.Add($"fp:{Float(light.FresnelPower)}");
            }
            else if (light.GlowTexture.Length > 0)
            {
                segments.Add($"glow:{light.GlowTexture}");
            }
            if (light.EmissiveEnabled && light.EmissiveColor != 0 && light.EmissiveMult != 0f)
            {
                segments.Add($"em:{Hex(light.EmissiveColor)}");
                segments.Add($"emm:{Float(light.EmissiveMult)}");
            }

            if (parts.TwoSided) segments.Add("ts");
            if (parts.SourceKey != null) segments.Add($"src:{parts.SourceKey}");
            return string.Join('|', segments);
        }

        public static string ForDefinition(MaterialDef def, string diffuse, string normal) => Encode(new MaterialNameParts(
            Diffuse: diffuse,
            Normal: normal,
            AlphaToken: def.AlphaToken,
            Effect: def.IsEffect,
            Decal: def.Decal,
            Lighting: CleanLighting(def.Lighting),
            TwoSided: def.TwoSided,
            SourceKey: def.Key,
            EffectFields: def.Effect is { } effect ? EffectParams.FromMaterial(effect).WithTexturePaths(CleanPath) : null,
            Env: def.Env?.WithTexturePaths(CleanPath)));

        private static void AddEffectTokens(List<string> segments, EffectParams effect)
        {
            if (effect.BlendMode != EffectParams.BlendAlpha) segments.Add($"bm:{effect.BlendMode}");
            segments.Add($"bc:{Hex(effect.BaseColor)}");
            segments.Add($"bs:{Float(effect.BaseColorScale)}");
            if (effect.Falloff is { } f)
                segments.Add($"fo:{Float(f.StartAngle)},{Float(f.StopAngle)},{Float(f.StartOpacity)},{Float(f.StopOpacity)},{(f.ScalesColor ? 1 : 0)}");
            if (effect.LightingInfluence is { } influence) segments.Add($"li:{Float(influence)}");
            if (effect.HasPalette)
                segments.Add($"pal:{(effect.PaletteColor ? "c" : "")}{(effect.PaletteAlpha ? "a" : "")}:{effect.PaletteTexture}");
        }

        private static void AddEnvironmentTokens(List<string> segments, EnvironmentParams env)
        {
            segments.Add($"env:{env.CubeTexture}");
            if (env.MaskTexture.Length > 0) segments.Add($"envk:{env.MaskTexture}");
            segments.Add($"envs:{Float(env.MaskScale)}");
        }

        public static string CleanPath(string path) => NifConverter.CleanTexturePathForDeploy(path, isDiffuse: false);

        public static LightingParams CleanLighting(LightingParams lighting) =>
            lighting.WithTexturePaths(CleanPath);

        private static string Hex(int rgb) => rgb.ToString("X6", CultureInfo.InvariantCulture);

        // Rounding then "+ 0f" folds tiny negatives (e.g. -4e-8) and negative zero into 0 so "-0" never reaches the name.
        private static string Float(float value) => (Math.Round((double)value, 4, MidpointRounding.AwayFromZero) + 0d).ToString("0.####", CultureInfo.InvariantCulture);
    }
}
