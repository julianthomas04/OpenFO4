using System.Text.Json.Nodes;
using Mutagen.Bethesda.Fallout4;

namespace AssetConverter
{
    /// <summary>The SPGD DATA fields the engine reads, with snake_case names. <c>type</c> is the particle type enum value (0 rain, 1 snow).</summary>
    internal static class ShaderParticleGeometryJson
    {
        public static JsonObject Data(IShaderParticleGeometryGetter record) => new()
        {
            ["type"] = (int)record.Type,
            ["gravity_velocity"] = record.GravityVelocity,
            ["rotation_velocity"] = record.RotationVelocity,
            ["particle_size_x"] = record.ParticleSizeX,
            ["particle_size_y"] = record.ParticleSizeY,
            ["center_offset_min"] = record.CenterOffsetMin,
            ["center_offset_max"] = record.CenterOffsetMax,
            ["initial_rotation"] = record.InitialRotation,
            ["subtextures_x"] = (int)Math.Round(record.NumSubtexturesX),
            ["subtextures_y"] = (int)Math.Round(record.NumSubtexturesY),
            ["box_size"] = record.BoxSize,
            ["particle_density"] = record.ParticleDensity
        };

        /// <summary>
        /// The particle material's draw state: blend enable and factors (named as in <c>material.extras.effect</c>),
        /// base colour as 0..255 bytes, depth write and effect lighting with its influence.
        /// </summary>
        public static JsonObject MaterialData(EffectMaterialParams material)
        {
            var (blendEnabled, source, destination) = BaseMaterialFile.BlendFactorsOf(material.Blend);
            return new JsonObject
            {
                ["blend_enabled"] = blendEnabled,
                ["blend_src"] = EffectMaterialExtras.BlendFactors[source],
                ["blend_dst"] = EffectMaterialExtras.BlendFactors[destination],
                ["base_color"] = new JsonArray(Byte(material.BaseColor.X), Byte(material.BaseColor.Y), Byte(material.BaseColor.Z)),
                ["depth_write"] = material.ZWrite,
                ["effect_lighting"] = material.EffectLightingEnabled,
                ["lighting_influence"] = material.LightingInfluence
            };
        }

        private static int Byte(float channel) => (int)MathF.Round(Math.Clamp(channel, 0f, 1f) * 255f);
    }
}
