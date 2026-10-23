using AssetConverter.Abc;
using Xunit;

namespace InstallerTests
{
    public sealed class AbcReaderTests
    {
        /// <summary>The ABC version and empty int, uint and double pools, then <paramref name="rest"/>.</summary>
        private static byte[] AfterNumberPools(params byte[] rest) => [0x10, 0x00, 0x2E, 0x00, 0, 0, 0, .. rest];

        [Fact]
        public void Read_Pools_KeepTheirValuesAtTheirIndices()
        {
            var builder = new AbcBuilder();
            int big = builder.Int(-70000);
            int unsigned = builder.UInt(4000000000);
            int ratio = builder.Double(0.125);
            int name = builder.Name("Widget", "Pkg.Sub");

            var abc = AbcReader.Read(builder.Build());

            Assert.Equal(-70000, abc.Ints[big]);
            Assert.Equal(4000000000u, abc.UInts[unsigned]);
            Assert.Equal(0.125, abc.Doubles[ratio]);
            Assert.Equal("Pkg.Sub.Widget", abc.QualifiedName(name));
            Assert.Equal("Widget", abc.LocalName(name));
        }

        [Fact]
        public void Read_PrivateName_IsQualifiedByItsLocalNameOnly()
        {
            var builder = new AbcBuilder();
            int name = builder.Name("secret", "Pkg:Widget", AbcBuilder.PrivateNamespace);

            Assert.Equal("secret", AbcReader.Read(builder.Build()).QualifiedName(name));
        }

        [Fact]
        public void Read_ClassesTraitsAndBodies_AreKept()
        {
            var builder = new AbcBuilder();
            int method = builder.Function(new AbcCode().ReturnVoid().ToArray());
            builder.Class("Pkg", "Widget", builder.Name("Base", "Pkg"), builder.Method(), builder.Method(),
                [new AbcMethodTraitSpec(builder.Name("Run"), method), new AbcSlotSpec(builder.Name("count"), true, builder.Int(3), AbcBuilder.ConstantInt)]);

            var abc = AbcReader.Read(builder.Build());

            var instance = Assert.Single(abc.Instances);
            Assert.Equal("Pkg.Widget", abc.QualifiedName(instance.Name));
            Assert.Equal("Pkg.Base", abc.QualifiedName(instance.SuperName));
            Assert.Equal([AbcTraitKind.Method, AbcTraitKind.Const], instance.Traits.Select(t => t.Kind));
            Assert.Equal(new AbcValueRef(AbcBuilder.ConstantInt, 1), instance.Traits[1].Default);
            Assert.Equal([0x47], Assert.Single(abc.Bodies).Code);
        }

        [Fact]
        public void Read_Truncated_Throws()
        {
            byte[] abc = new AbcBuilder().Build();

            Assert.Throws<AbcFormatException>(() => AbcReader.Read(abc[..^2]));
        }

        [Fact]
        public void Read_UnknownNamespaceKind_Throws()
        {
            Assert.Throws<AbcFormatException>(() => AbcReader.Read(AfterNumberPools(0, 2, 0x42, 0)));
        }

        [Fact]
        public void Read_UnknownMultinameKind_Throws()
        {
            Assert.Throws<AbcFormatException>(() => AbcReader.Read(AfterNumberPools(0, 0, 0, 2, 0x42)));
        }

        [Fact]
        public void Read_PoolIndexOutsideItsPool_Throws()
        {
            Assert.Throws<AbcFormatException>(() => AbcReader.Read(AfterNumberPools(0, 2, 0x16, 5)));
        }

        [Fact]
        public void Read_UnknownTraitKind_Throws()
        {
            Assert.Throws<AbcFormatException>(() => AbcReader.Read(AfterNumberPools(0, 0, 0, 0, 0, 0, 0, 1, 0, 1, 0, 7)));
        }

        [Fact]
        public void Read_ValueBeyondThirtyBits_Throws()
        {
            Assert.Throws<AbcFormatException>(() => AbcReader.Read(AfterNumberPools(0xFF, 0xFF, 0xFF, 0xFF, 0x0F)));
        }
    }
}
