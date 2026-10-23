using System.Reflection;
using AssetConverter;
using InstallerTests.Nif;
using NiflySharp.Blocks;
using NiflySharp.Structs;
using Xunit;

namespace InstallerTests
{
    // WaterShaderParams reads NiflySharp's private BSWaterShaderProperty fields by reflection; these tests pin the layout.
    public class WaterShaderParamsTests
    {
        const BindingFlags PrivateInstance = BindingFlags.NonPublic | BindingFlags.Instance;

        [Fact]
        public void ReflectedFieldNames_AllExistOnTheRealNiflySharpType()
        {
            var missing = WaterShaderParams.ReflectedFieldNames
                .Where(name => typeof(BSWaterShaderProperty).GetField(name, PrivateInstance) == null);

            Assert.Equal(Array.Empty<string>(), missing);
        }

        [Fact]
        public void TexCoordFields_AreTexCoordsWithPublicUAndV()
        {
            foreach (var name in new[] { WaterShaderParams.UvOffsetField, WaterShaderParams.UvScaleField })
            {
                var type = typeof(BSWaterShaderProperty).GetField(name, PrivateInstance)!.FieldType;
                Assert.Equal(typeof(TexCoord), type);
                Assert.NotNull(type.GetProperty(WaterShaderParams.TexCoordU));
                Assert.NotNull(type.GetProperty(WaterShaderParams.TexCoordV));
            }
        }

        [Fact]
        public void FromShader_RealShader_ReadsEveryField()
        {
            var shader = NifSceneBuilder.CreateWaterShader(452, 0x80000008, 1, new TexCoord(0.5f, -0.25f), new TexCoord(2f, 3f));

            var parameters = WaterShaderParams.FromShader(shader);

            Assert.Equal(new WaterShaderParams(452, 0x80000008, 1, 0.5f, -0.25f, 2f, 3f), parameters);
        }

        [Fact]
        public void Read_ShaderLayoutWithoutTheExpectedFields_ThrowsWaterShaderUnreadable()
        {
            var ex = Assert.Throws<WaterShaderUnreadableException>(() => WaterShaderParams.Read(new object(), typeof(object)));

            Assert.Contains(WaterShaderParams.WaterFlagsField, ex.Message);
            Assert.IsType<InvalidDataException>(ex.InnerException);
        }
    }
}
