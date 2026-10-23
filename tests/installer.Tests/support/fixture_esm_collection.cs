using Xunit;

namespace InstallerTests
{
    /// <summary>One plugin load shared by every test class that asserts on the loaded database.</summary>
    [CollectionDefinition(Name)]
    public sealed class FixtureEsm : ICollectionFixture<LoadedFixtureDatabase>
    {
        public const string Name = "Fixture ESM database";
    }

    public sealed class PlacedRow
    {
        public required string FormKey { get; init; }
        public string? CellFormKey { get; init; }
        public string? BaseObject { get; init; }
        public string? ObjectType { get; init; }
        public long IsPersistent { get; init; }
        public double PositionX { get; init; }
        public double PositionY { get; init; }
        public double PositionZ { get; init; }
        public double RotationX { get; init; }
        public double RotationY { get; init; }
        public double RotationZ { get; init; }
        public double Scale { get; init; }
        public long IsInitiallyDisabled { get; init; }
        public string? EnableParent { get; init; }
        public long EnableParentOpposite { get; init; }
        public long InitiallyHidden { get; init; }
        public long HasPrimitive { get; init; }
        public string? MaterialSwap { get; init; }
        public double? RadiusOverride { get; init; }
        public double? LightFovPm { get; init; }
        public double? LightFadePm { get; init; }
        public double? LightEndDistanceCap { get; init; }
        public double? LightShadowDepthBias { get; init; }
        public double? LightNearClip { get; init; }
        public double? LightVolumetricIntensity { get; init; }
        public long? RecordFlags { get; init; }
        public byte[]? RagdollPose { get; init; }
    }

    public sealed class CellRow
    {
        public required string FormKey { get; init; }
        public string? EditorId { get; init; }
        public string? Worldspace { get; init; }
        public long? GridX { get; init; }
        public long? GridY { get; init; }
        public long IsInterior { get; init; }
        public long IsWorldspacePersistentCell { get; init; }
        public string? Flags { get; init; }
        public string? Lighting { get; init; }
    }
}
