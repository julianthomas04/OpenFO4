using AssetConverter;
using AssetConverter.Audio;
using AssetConverter.Pipeline;
using AssetConverter.ShaderTranslation;
using AssetConverter.UiLayout;
using AssetConverter.Video;
using Xunit;

namespace InstallerTests.Pipeline
{
    /// <summary>
    /// Developer guard: ties each converter's own version constant to the revision of the asset group it produces. Changing a constant
    /// changes converted output, so the group's revision must be bumped with it, otherwise installs made earlier keep the stale output.
    /// </summary>
    public sealed class ConverterRevisionPinsTests
    {
        public static TheoryData<string, string, long, long, int> Pins() => new()
        {
            { AssetGroups.Id(AssetGroup.Shaders), nameof(ShaderTranslatorVersion), ShaderTranslatorVersion.Value, 2, 1 },
            { AssetGroups.Id(AssetGroup.Meshes), nameof(NifNodeExtras), NifNodeExtras.Version, 1, 1 },
            { AssetGroups.Id(AssetGroup.Meshes), nameof(ParticleSystemExtras), ParticleSystemExtras.Version, 1, 1 },
            { AssetGroups.Id(AssetGroup.Meshes), nameof(EffectMaterialExtras), EffectMaterialExtras.Version, 1, 1 },
            { AssetGroups.Id(AssetGroup.Collision), nameof(CollisionSidecars), CollisionSidecars.ExtractorVersion, 2, 1 },
            { AssetGroups.Id(AssetGroup.Audio), nameof(AudioConversion), AudioConversion.ConverterVersion, 1, 1 },
            { AssetGroups.Id(AssetGroup.Video), nameof(VideoConversion), VideoConversion.ConverterVersion, 1, 1 },
            { AssetGroups.Id(AssetGroup.Interface), nameof(FontExtractor), FontExtractor.ExtractorVersion, 1, 1 },
            { AssetGroups.Id(AssetGroup.Interface), nameof(PipboyArtExtractor), PipboyArtExtractor.ExtractorVersion, 4, 1 },
            { AssetGroups.Id(AssetGroup.Interface), nameof(UiDocumentHeader), UiDocumentHeader.ExtractorVersion, 5, 1 },
            { AssetGroups.Id(AssetGroup.TexturesDatabase), nameof(StringTables), StringTables.ExtractorVersion, 1, 2 },
        };

        [Theory]
        [MemberData(nameof(Pins))]
        public void AConverterVersionAndItsGroupRevisionChangeTogether(string groupId, string converter, long version, long pinnedVersion, int pinnedRevision)
        {
            Assert.True(AssetGroups.TryFromId(groupId, out var group));
            Assert.True(
                version == pinnedVersion && AssetGroups.CurrentRevisions[group] == pinnedRevision,
                $"{converter} is at version {version} (pinned {pinnedVersion}) and the '{AssetGroups.Id(group)}' group at revision {AssetGroups.CurrentRevisions[group]} (pinned {pinnedRevision}). " +
                $"A converter version change alters the converted output: bump the '{AssetGroups.Id(group)}' revision in AssetGroups and update this pin to the new version and revision.");
        }
    }
}
