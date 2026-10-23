using Xunit;

namespace InstallerTests
{
    // Behaviour: the engine reads
    // these rows through src/engine/src/assets/db/placed_objects.rs.
    [Collection(FixtureEsm.Name)]
    public class EsmLoadPlacedObjectTests(LoadedFixtureDatabase db)
    {
        PlacedRow Placed(string editorId) =>
            db.QuerySingle<PlacedRow>("SELECT * FROM placed_objects WHERE form_key = @key", new { key = db.KeyOf(editorId) });

        bool IsImported(string editorId) =>
            db.QuerySingle<long>("SELECT COUNT(*) FROM placed_objects WHERE form_key = @key", new { key = db.KeyOf(editorId) }) == 1;

        [Fact]
        public void PlacedObject_PlainRef_StoresRawPlacementScaleAndOwners()
        {
            var row = Placed("PlainRef");

            Assert.Equal(db.KeyOf("FixtureExterior"), row.CellFormKey);
            Assert.Equal(db.KeyOf("WallStatic"), row.BaseObject);
            Assert.StartsWith("PlacedObject", row.ObjectType);
            Assert.Equal(0, row.IsPersistent);
            Assert.Equal((100.0, 200.0, 300.0), (row.PositionX, row.PositionY, row.PositionZ));
            Assert.Equal(0.1, row.RotationX, 6);
            Assert.Equal(0.2, row.RotationY, 6);
            Assert.Equal(0.3, row.RotationZ, 6);
            Assert.Equal(2.5, row.Scale);
        }

        [Fact]
        public void PlacedObject_WithoutScaleSubrecord_DefaultsScaleToOne() =>
            Assert.Equal(1.0, Placed("UnscaledRef").Scale);

        [Fact]
        public void PlacedObject_DeletedRef_IsNotImported() =>
            Assert.False(IsImported("DeletedRef"));

        [Fact]
        public void PlacedObject_RefWithoutBase_IsNotImported() =>
            Assert.False(IsImported("BaselessRef"));

        [Fact]
        public void PlacedObject_RefIsRegisteredInRecordsTable()
        {
            var type = db.QuerySingle<string>("SELECT record_type FROM records WHERE form_key = @key", new { key = db.KeyOf("PlainRef") });

            Assert.Equal("REFR", type);
        }

        [Fact]
        public void PlacedObject_InitiallyDisabledFlag_SetsDisabledAndHidden()
        {
            var row = Placed("DisabledRef");

            Assert.Equal((1, 1), (row.IsInitiallyDisabled, row.InitiallyHidden));
        }

        [Fact]
        public void PlacedObject_EnableParent_StoresParentAndOppositeFlag()
        {
            var follows = Placed("FollowsOffRoot");
            var inverts = Placed("InvertsOffRoot");

            Assert.Equal((db.KeyOf("OffRoot"), 0), (follows.EnableParent, follows.EnableParentOpposite));
            Assert.Equal((db.KeyOf("OffRoot"), 1), (inverts.EnableParent, inverts.EnableParentOpposite));
        }

        // Expected verdicts follow the rule in the docs: a child mirrors its parent's enable state, inverted by the
        // opposite flag; its own InitiallyDisabled flag is ignored; the player ref and unknown parents are enabled;
        // an enable-parent cycle resolves as visible.
        [Theory]
        [InlineData("OffRoot", true)]
        [InlineData("OnRoot", false)]
        [InlineData("FollowsOffRoot", true)]
        [InlineData("InvertsOffRoot", false)]
        [InlineData("FollowsOnRoot", false)]
        [InlineData("InvertsOnRoot", true)]
        [InlineData("InvertsTheInverter", true)]
        [InlineData("OwnDisabledFlagIgnoredUnderParent", false)]
        [InlineData("FollowsPlayer", false)]
        [InlineData("InvertsPlayer", true)]
        [InlineData("CycleA", false)]
        [InlineData("CycleB", false)]
        public void PlacedObject_EnableChain_ResolvesInitiallyHidden(string editorId, bool expectedHidden) =>
            Assert.Equal(expectedHidden, Placed(editorId).InitiallyHidden == 1);

        [Fact]
        public void PlacedObject_RefOfMarkerStatic_IsLinkedToMarkerBase()
        {
            var baseKey = Placed("MarkerRef").BaseObject;
            var isMarker = db.QuerySingle<long>("SELECT is_marker FROM statics WHERE form_key = @key", new { key = baseKey });

            Assert.Equal(1, isMarker);
        }

        [Fact]
        public void PlacedObject_LightRef_KeepsRadiusLightingBlockAndRecordFlags()
        {
            var row = Placed("LampRef");

            Assert.Equal(25.0, row.RadiusOverride);
            Assert.Equal(5.0, row.LightFovPm);
            Assert.Equal(0.25, row.LightFadePm);
            Assert.Equal(4000.0, row.LightEndDistanceCap);
            Assert.Equal(0.5, row.LightShadowDepthBias);
            Assert.Equal(12.0, row.LightNearClip);
            Assert.Equal(0.75, row.LightVolumetricIntensity);
            Assert.Equal(0x200 | 0x10000, row.RecordFlags);
        }

        [Fact]
        public void PlacedObject_NonLightRef_LeavesLightColumnsNull()
        {
            var row = Placed("PlainRef");

            Assert.Null(row.RadiusOverride);
            Assert.Null(row.LightFovPm);
            Assert.Null(row.LightVolumetricIntensity);
        }

        [Fact]
        public void PlacedObject_RagdollData_IsSerialisedToTwentyEightByteEntries()
        {
            var pose = Placed("PosedRef").RagdollPose!;

            Assert.Equal(2 * 28, pose.Length);
            var secondEntry = pose.AsSpan(28, 28);
            Assert.Equal(7, secondEntry[0]);
            Assert.Equal(new[] { -4f, 5f, 6f }, ReadFloats(secondEntry.Slice(4, 12)));
            Assert.Equal(new[] { 0f, 1f, 0f }, ReadFloats(secondEntry.Slice(16, 12)));
        }

        [Fact]
        public void PlacedObject_WithoutRagdollData_LeavesPoseNull() =>
            Assert.Null(Placed("PlainRef").RagdollPose);

        [Fact]
        public void PlacedObject_PlacedNpc_StoresScaleBaseAndTypeName()
        {
            var row = Placed("ScaledNpc");

            Assert.StartsWith("PlacedNpc", row.ObjectType);
            Assert.Equal("000ABC:Fallout4.esm", row.BaseObject);
            Assert.Equal(1.25, row.Scale);
            Assert.Equal((7.0, 8.0, 9.0), (row.PositionX, row.PositionY, row.PositionZ));
        }

        [Fact]
        public void PlacedObject_RefInPersistentGroup_IsMarkedPersistent()
        {
            var row = Placed("WorldPersistentRef");

            Assert.Equal(1, row.IsPersistent);
            Assert.Equal(db.KeyOf("FixtureWorldTopCell"), row.CellFormKey);
        }

        [Fact]
        public void PlacedObject_MaterialSwapLink_IsStoredFromRawScan() =>
            Assert.Equal(db.KeyOf("RedSwap"), Placed("SwappedRef").MaterialSwap);

        [Fact]
        public void PlacedObject_WithoutMaterialSwapLink_LeavesColumnNull() =>
            Assert.Null(Placed("PlainRef").MaterialSwap);

        static float[] ReadFloats(ReadOnlySpan<byte> bytes) =>
            [BitConverter.ToSingle(bytes[..4]), BitConverter.ToSingle(bytes[4..8]), BitConverter.ToSingle(bytes[8..12])];
    }
}
