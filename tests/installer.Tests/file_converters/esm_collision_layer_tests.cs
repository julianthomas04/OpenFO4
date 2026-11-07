using AssetConverter;
using Dapper;
using Microsoft.Data.Sqlite;
using Mutagen.Bethesda.Fallout4;
using Mutagen.Bethesda.Plugins;
using Xunit;

namespace InstallerTests
{
    public class EsmCollisionLayerTests
    {
        static readonly FormKey Other = FormKey.Factory("0E7518:Fallout4.esm");

        static CollisionLayer LayerCollidingWith(params IFormLinkGetter<ICollisionLayerGetter>[] links)
        {
            var layer = new CollisionLayer(FormKey.Factory("000800:Fixture.esm"), Fallout4Release.Fallout4);
            layer.CollidesWith = [.. links.Select(link => new FormLink<ICollisionLayerGetter>(link.FormKey))];
            return layer;
        }

        [Fact]
        public void CollisionLayerFlags_MatchTheRawCollFlagBits()
        {
            Assert.Equal(
                (1, 2, 4),
                ((int)CollisionLayer.Flag.TriggerVolume, (int)CollisionLayer.Flag.Sensor, (int)CollisionLayer.Flag.NavmeshObstacle));
        }

        [Fact]
        public void CollidesWithKeys_AreFormKeysInRecordOrder()
        {
            var second = FormKey.Factory("0E7519:Fallout4.esm");

            var keys = CollisionLayerProcessor.CollidesWithKeys(LayerCollidingWith(new FormLink<ICollisionLayerGetter>(second), new FormLink<ICollisionLayerGetter>(Other)));

            Assert.Equal(["0E7519:Fallout4.esm", "0E7518:Fallout4.esm"], keys);
        }

        [Fact]
        public void CollidesWithKeys_ForAnEmptyList_AreEmptyNotAbsent()
        {
            Assert.Empty(CollisionLayerProcessor.CollidesWithKeys(LayerCollidingWith())!);
        }

        [Fact]
        public void CollidesWithKeys_WithoutAList_AreAbsent()
        {
            var layer = new CollisionLayer(FormKey.Factory("000800:Fixture.esm"), Fallout4Release.Fallout4) { CollidesWith = null };

            Assert.Null(CollisionLayerProcessor.CollidesWithKeys(layer));
        }

        [Fact]
        public void CollidesWithKeys_WithANullLink_ThrowsAReadFailureThatTheTallyRecognises()
        {
            var layer = LayerCollidingWith(new FormLink<ICollisionLayerGetter>(Other));
            layer.CollidesWith!.Add(new FormLink<ICollisionLayerGetter>(FormKey.Null));

            var failure = Assert.Throws<InvalidDataException>(() => CollisionLayerProcessor.CollidesWithKeys(layer));

            Assert.True(ReadFailureTally.IsReadFailure(failure));
        }

        [Fact]
        public async Task DoorProcessor_WithoutObjectBounds_StoresNullBounds()
        {
            var ct = TestContext.Current.CancellationToken;
            using var conn = new SqliteConnection("Data Source=:memory:");
            await conn.OpenAsync(ct);
            await conn.ExecuteAsync(await File.ReadAllTextAsync(InstallerPaths.Require("src/installer/file_converters/esm_schema.sql"), ct));
            var door = new Door(FormKey.Factory("000801:Fixture.esm"), Fallout4Release.Fallout4) { ObjectBounds = null! };

            await new DoorProcessor().ProcessRecordsAsync([door], "DOOR", conn, silent: true, ct: ct);

            Assert.Null(conn.QuerySingle<string?>("SELECT bounds FROM doors"));
        }
    }
}
