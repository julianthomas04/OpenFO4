using System.Globalization;
using AssetConverter;
using Xunit;

namespace InstallerTests
{
    public class ByteFormatTests
    {
        [Theory]
        [InlineData(0L, "0 B")]
        [InlineData(1023L, "1023 B")]
        [InlineData(1024L, "1.0 KB")]
        [InlineData(1536L, "1.5 KB")]
        [InlineData(1048576L, "1.0 MB")]
        [InlineData(5L * 1048576 + 524288, "5.5 MB")]
        [InlineData(1073741824L, "1.0 GB")]
        [InlineData(3L * 1073741824 + 536870912, "3.5 GB")]
        [InlineData(-5L, "-5 B")]
        public void Format_BinaryUnitBoundaries_PicksLargestFittingUnit(long bytes, string expected)
        {
            var previous = CultureInfo.CurrentCulture;
            CultureInfo.CurrentCulture = CultureInfo.InvariantCulture;
            try
            {
                Assert.Equal(expected, ByteFormat.Format(bytes));
            }
            finally
            {
                CultureInfo.CurrentCulture = previous;
            }
        }

        [Fact]
        public void Format_UnderACommaDecimalCulture_StillUsesADecimalPoint()
        {
            var previous = CultureInfo.CurrentCulture;
            CultureInfo.CurrentCulture = new CultureInfo("de-DE");
            try
            {
                Assert.Equal("1.5 KB", ByteFormat.Format(1536));
            }
            finally
            {
                CultureInfo.CurrentCulture = previous;
            }
        }
    }
}
