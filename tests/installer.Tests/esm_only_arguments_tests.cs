using AssetConverter;
using Xunit;

namespace InstallerTests
{
    public class EsmOnlyArgumentsTests
    {
        [Fact]
        public void TryParse_SubcommandOnly_MeansFullRebuild()
        {
            bool ok = EsmOnlyArguments.TryParse(["esm-only"], out var tables);

            Assert.True(ok);
            Assert.Null(tables);
        }

        [Fact]
        public void TryParse_TablesFlagWithSeparateValue_ReturnsTheRawList()
        {
            bool ok = EsmOnlyArguments.TryParse(["esm-only", "--tables", "statics,lights"], out var tables);

            Assert.True(ok);
            Assert.Equal("statics,lights", tables);
        }

        [Fact]
        public void TryParse_TablesFlagWithEqualsSign_ReturnsTheRawList()
        {
            bool ok = EsmOnlyArguments.TryParse(["esm-only", "--tables=statics,lights"], out var tables);

            Assert.True(ok);
            Assert.Equal("statics,lights", tables);
        }

        [Fact]
        public void TryParse_EqualsFormWithEmptyValue_ReturnsEmptyListForThePlanToReject()
        {
            bool ok = EsmOnlyArguments.TryParse(["esm-only", "--tables="], out var tables);

            Assert.True(ok);
            Assert.Equal("", tables);
        }

        [Theory]
        [InlineData("esm-only", "--tables")]
        [InlineData("esm-only", "statics")]
        [InlineData("esm-only", "--table", "statics")]
        [InlineData("esm-only", "--tables", "statics", "lights")]
        [InlineData("esm-only", "--tables=statics", "extra")]
        public void TryParse_MalformedCommandLine_IsRejected(params string[] args)
        {
            bool ok = EsmOnlyArguments.TryParse(args, out var tables);

            Assert.False(ok);
            Assert.Null(tables);
        }

        [Theory]
        [InlineData("statics", new[] { "statics" })]
        [InlineData("statics,lights", new[] { "statics", "lights" })]
        [InlineData(" statics , lights ", new[] { "statics", "lights" })]
        [InlineData("statics,,lights,", new[] { "statics", "lights" })]
        [InlineData("", new string[0])]
        public void SplitTables_TrimsAndDropsEmptyEntries(string csv, string[] expected) =>
            Assert.Equal(expected, EsmOnlyArguments.SplitTables(csv));
    }
}
