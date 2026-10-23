using System.Text;
using System.Text.Json.Nodes;
using AssetConverter;
using Xunit;

namespace InstallerTests.FileConverters
{
    public class GltfMaterialExtrasTests
    {
        [Theory]
        [InlineData("", 0x02CC5D05u)]
        [InlineData("a", 0x550D7456u)]
        [InlineData("abc", 0x32D153FFu)]
        [InlineData("Nobody inspects the spammish repetition", 0xE2293B2Fu)]
        public void Xxh32_MatchesThePublishedSeedZeroVectors(string input, uint expected)
        {
            Assert.Equal(expected, Xxh32.Hash(Encoding.UTF8.GetBytes(input)));
        }

        [Fact]
        public void CacheKey_SameNameDifferentExtras_GivesDistinctKeys()
        {
            var first = new JsonObject { ["blend"] = "add" };
            var second = new JsonObject { ["blend"] = "mul" };

            Assert.NotEqual(GltfMaterialExtras.CacheKey("same", first), GltfMaterialExtras.CacheKey("same", second));
        }

        [Fact]
        public void CacheKey_EqualExtras_GivesEqualKeysAndNoExtrasKeepsTheName()
        {
            Assert.Equal(
                GltfMaterialExtras.CacheKey("m", new JsonObject { ["a"] = 1 }),
                GltfMaterialExtras.CacheKey("m", new JsonObject { ["a"] = 1 }));
            Assert.Equal("m", GltfMaterialExtras.CacheKey("m", null));
        }
    }
}
