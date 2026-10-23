using System.Text.Json.Nodes;
using NiflySharp;
using NiflySharp.Blocks;

namespace AssetConverter
{
    /// <summary>The glTF material a particle system draws with: its name and the <c>effect</c> extras.</summary>
    internal sealed record ParticleMaterial(string Name, JsonObject Extras);

    /// <summary>Builds the effect material of a <see cref="NiParticleSystem"/> from its effect shader, through the same technique selection as effect-shader meshes.</summary>
    internal static class ParticleMaterialConversion
    {
        public const string WithMaterialCounter = "particle systems drawn through an effect material";
        public const string LeftOutPrefix = "particle systems left out without an effect material: ";
        public const string NoEffectShaderReason = "no effect shader";
        public const string NoParticleDataReason = "no particle data";

        public static ParticleMaterial? TryBuild(
            NifFile nif,
            NiParticleSystem system,
            IEffectShaderCatalog? catalog,
            Func<string, string> deployTexture,
            Action<string> reportWarning)
        {
            if (catalog == null) return LeftOut(EffectCounters.NoCatalogReason);
            if (!NifControllerExtractor.TryGetBlock<BSEffectShaderProperty>(nif, system.ShaderPropertyRef.Index, "particle effect shader", out var shader))
                return LeftOut(NoEffectShaderReason);
            if (!NifControllerExtractor.TryGetBlock<NiPSysData>(nif, system.Data_NiBlockRef_NPSD.Index, "particle data", out var particleData))
                return LeftOut(NoParticleDataReason);

            try
            {
                var data = EffectShaderData.Read(shader);
                var definition = ResolveMaterialFile(shader);
                var material = definition?.Effect;
                var alpha = EffectShapeConversion.ReadAlpha(nif, system.HasAlphaProperty, system.AlphaPropertyRef.Index);
                var facts = EffectShapeFacts.Create(data, alpha, material, EffectGeometry.OfParticles(particleData));
                var selection = EffectTechnique.Select(facts, catalog);
                if (selection.IsHidden) return LeftOut(selection.HiddenReason!);

                var tracks = ShaderControllerTracks.ForShader(nif, system.ShaderPropertyRef.Index).Tracks;
                var extras = EffectMaterialExtras.Build(data, alpha, material, selection, tracks, deployTexture);
                return new ParticleMaterial(Name(data, definition, extras, deployTexture), extras);
            }
            catch (Exception ex) when (ex is EffectShaderUnreadableException or NifAnimationException)
            {
                reportWarning($"particle system '{system.Name?.String}' left out: {ex.Message}");
                return LeftOut(EffectCounters.UnreadableReason);
            }
        }

        private static MaterialDef? ResolveMaterialFile(BSEffectShaderProperty shader)
        {
            string? reference = NifConverter.GetEffectBgemReference(shader);
            return reference != null && MaterialCensus.TryResolve(reference, out var definition) ? definition : null;
        }

        private static string Name(EffectShaderData data, MaterialDef? definition, JsonObject extras, Func<string, string> deployTexture) =>
            MaterialNameCodec.Encode(new MaterialNameParts(
                Diffuse: data.SourceTexture.Length > 0 ? deployTexture(data.SourceTexture) : "",
                Normal: "",
                AlphaToken: "",
                Effect: true,
                Decal: false,
                Lighting: LightingParams.None,
                TwoSided: false,
                SourceKey: definition?.Key,
                ExtrasKey: GltfMaterialExtras.KeyOf(extras)));

        private static ParticleMaterial? LeftOut(string reason)
        {
            NifAnimationStats.Add(LeftOutPrefix + reason);
            return null;
        }
    }
}
