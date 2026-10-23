using AssetConverter;
using Xunit;

namespace InstallerTests.Nif
{
    [Collection(StaticStateCollections.NifConverterState)]
    public class NifAnimationStatsTests
    {
        public static TheoryData<string> EffectCounters() => new()
        {
            ClipTiming.RangeReplacedCounter,
            ControllerSources.NonFiniteKeysCounter,
            StandaloneTransformControllers.WithoutKeysCounter,
            StandaloneTransformControllers.InvalidKeysCounter,
            ShaderControllerTracks.SequenceTiesCounter,
            ShaderControllerTracks.WithoutKeysCounter,
            NifNodeExtras.OffsetTiesCounter,
            NifNodeExtras.OffsetAllZeroCounter,
            NifNodeExtras.OffsetDroppedStandaloneCounter,
            ParticleSystemExtras.MissingObjectCounter,
            ParticleSystemExtras.NonFiniteSystemCounter,
            ParticleSystemExtras.NonFiniteControllerCounter,
            ParticleSystemExtras.EmitterWithoutControllerCounter,
            ParticleSystemExtras.ControllerWithoutKeysCounter,
            ParticleSystemExtras.ControllerTimingMismatchCounter,
            ParticleSystemExtras.MeshEmitterVelocityCounter,
            SystemControllers.SequenceTiesCounter,
            NifEffectNodes.MissingReferenceCounter,
            NifEffectNodes.AmbiguousReferenceCounter,
            NifConverter.EmitterSourcePointsCounter,
            ParticleMaterialConversion.WithMaterialCounter,
            global::AssetConverter.EffectCounters.WithExtras,
            global::AssetConverter.EffectCounters.HiddenPrefix + global::AssetConverter.EffectCounters.ProceduralLightningReason,
            global::AssetConverter.EffectCounters.RefractionShapes,
            global::AssetConverter.EffectCounters.RefractionSkippedPrefix + global::AssetConverter.EffectCounters.UnreadableReason,
        };

        [Theory]
        [MemberData(nameof(EffectCounters))]
        public void EffectPipelineCounter_IsPrintedInTheEffectSectionOnly(string counter)
        {
            NifAnimationStats.Add(counter);

            Assert.Contains(NifAnimationStats.EffectSummaryLines(), l => l.StartsWith(counter, StringComparison.Ordinal));
            Assert.DoesNotContain(NifAnimationStats.AnimationSummaryLines(), l => l.StartsWith(counter, StringComparison.Ordinal));
        }

        [Fact]
        public void UnsupportedParticleController_IsPrintedInTheEffectSection()
        {
            const string description = "particle controller NiVisController";
            NifAnimationStats.AddUnsupported(description);

            Assert.Contains(NifAnimationStats.EffectSummaryLines(), l => l.StartsWith(NifAnimationStats.UnsupportedPrefix + description, StringComparison.Ordinal));
        }
    }
}
