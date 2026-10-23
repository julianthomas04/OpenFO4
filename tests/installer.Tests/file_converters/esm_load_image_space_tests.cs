using System.Text.Json;
using Xunit;

namespace InstallerTests
{
    // Image space extraction contract: the water renderer reads the depth of field of a WATR's image space
    // (underwater fog).
    [Collection(FixtureEsm.Name)]
    public class EsmLoadImageSpaceTests(LoadedFixtureDatabase db)
    {
        [Fact]
        public void ImageSpace_DepthOfField_KeepsStrengthDistanceAndRange()
        {
            var json = db.QuerySingle<string>("SELECT data FROM image_spaces WHERE form_key = @key", new { key = db.KeyOf("FixtureImageSpace") });

            var depthOfField = JsonDocument.Parse(json).RootElement.GetProperty("DepthOfField");

            Assert.Equal(1f, depthOfField.GetProperty("Strength").GetSingle());
            Assert.Equal(-2500f, depthOfField.GetProperty("Distance").GetSingle());
            Assert.Equal(3500f, depthOfField.GetProperty("Range").GetSingle());
        }

        [Fact]
        public void ImageSpace_DepthOfField_ExportsTheRawModeFloatAndTheVignette()
        {
            var depthOfField = DepthOfField("FixtureImageSpace");

            Assert.Equal(LoadedFixtureDatabase.FixtureImageSpaceMode, depthOfField.GetProperty("Mode").GetSingle());
            Assert.Equal(LoadedFixtureDatabase.FixtureVignetteRadius, depthOfField.GetProperty("VignetteRadius").GetSingle());
            Assert.Equal(LoadedFixtureDatabase.FixtureVignetteStrength, depthOfField.GetProperty("VignetteStrength").GetSingle());
        }

        [Fact]
        public void ImageSpace_DepthOfFieldWithoutVignetteBytes_ExportsAZeroVignette()
        {
            var depthOfField = DepthOfField("FixtureImageSpaceWithoutVignette");

            Assert.Equal(LoadedFixtureDatabase.FixtureShortImageSpaceMode, depthOfField.GetProperty("Mode").GetSingle());
            Assert.Equal(0f, depthOfField.GetProperty("VignetteRadius").GetSingle());
            Assert.Equal(0f, depthOfField.GetProperty("VignetteStrength").GetSingle());
        }

        JsonElement DepthOfField(string editorId)
        {
            var json = db.QuerySingle<string>("SELECT data FROM image_spaces WHERE form_key = @key", new { key = db.KeyOf(editorId) });
            return JsonDocument.Parse(json).RootElement.GetProperty("DepthOfField");
        }
    }
}
