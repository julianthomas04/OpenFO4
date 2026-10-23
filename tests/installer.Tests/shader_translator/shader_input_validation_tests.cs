using AssetConverter.ShaderTranslation;
using AssetConverter.ShaderTranslation.Archive;
using AssetConverter.ShaderTranslation.Dxbc;
using AssetConverter.ShaderTranslation.Installation;
using InstallerTests.Archives;
using InstallerTests.Support;
using Xunit;
using static InstallerTests.ShaderTranslation.WgslTestKit;
using static InstallerTests.Support.Sm5Assembler;

namespace InstallerTests.ShaderTranslation
{
    public class Sm5OperandCountValidationTests
    {
        [Fact]
        public void Decode_InstructionWithTooManyOperands_ThrowsDecodeError()
        {
            var asm = Pixel().Temps(3).Instruction(Opcode.Mov, 0, Temp(0), Reg(1), Reg(2));

            var ex = Assert.Throws<Sm5DecodeException>(() => Sm5Decoder.Decode(asm.BuildDxbc()));

            Assert.Contains("mov takes 2 operand(s)", ex.Message);
        }

        [Fact]
        public void Decode_InstructionWithTooFewOperands_ThrowsDecodeError()
        {
            var asm = Pixel().Temps(3).Instruction(Opcode.Mad, 0, Temp(0), Reg(1), Reg(2));

            Assert.Throws<Sm5DecodeException>(() => Sm5Decoder.Decode(asm.BuildDxbc()));
        }

        [Fact]
        public void Decode_DeclarationMissingItsDataDwords_ThrowsDecodeError()
        {
            var asm = Pixel().Declaration(Opcode.DclTemps, 0);

            var ex = Assert.Throws<Sm5DecodeException>(() => Sm5Decoder.Decode(asm.BuildDxbc()));

            Assert.Contains("dcl_temps", ex.Message);
        }

        [Theory]
        [InlineData(Opcode.Sample, 4)]
        [InlineData(Opcode.SampleD, 6)]
        [InlineData(Opcode.Bfi, 5)]
        [InlineData(Opcode.ImmAtomicCmpExch, 5)]
        [InlineData(Opcode.Sync, 0)]
        public void ExpectedOperandCount_ListsTheArityOfTranslatedOpcodes(Opcode opcode, int expected)
        {
            Assert.Equal(expected, OpcodeInfo.ExpectedOperandCount(opcode));
        }

        [Fact]
        public void ExpectedOperandCount_IsNotTabulatedForUntranslatedOpcodes()
        {
            Assert.Null(OpcodeInfo.ExpectedOperandCount(Opcode.Lod));
        }
    }

    public class ShaderNameCollisionTests
    {
        private static string Names(params string[] names) =>
            "{\"shaders\":[" + string.Join(",", names.Select((n, i) => $"{{\"name\":\"{n}\",\"blob\":{i},\"stage\":\"vertex\"}}")) + "]}";

        [Fact]
        public void Parse_NamesDifferingOnlyInCase_Throw()
        {
            var ex = Assert.Throws<ShaderManifestException>(() => ShaderManifestReader.Parse(Names("Water_VS", "water_vs")));

            Assert.Contains("case-insensitively", ex.Message);
        }

        [Theory]
        [InlineData("con")]
        [InlineData("NUL")]
        [InlineData("Com3")]
        [InlineData("lpt9")]
        public void Parse_ReservedWindowsDeviceNames_Throw(string name)
        {
            Assert.Throws<ShaderManifestException>(() => ShaderManifestReader.Parse(Names(name)));
        }

        [Theory]
        [InlineData("water_ps", "water_ps_in")]
        [InlineData("water_ps", "WATER_PS_OUT")]
        public void Parse_NameThatEqualsAnotherShadersInterfaceStruct_Throws(string first, string second)
        {
            var ex = Assert.Throws<ShaderManifestException>(() => ShaderManifestReader.Parse(Names(first, second)));

            Assert.Contains("interface struct", ex.Message);
        }

        [Fact]
        public void Parse_DistinctNamesSharingAPrefix_AreAccepted()
        {
            var manifest = ShaderManifestReader.Parse(Names("water", "water_ps", "console", "inbound"));

            Assert.Equal(4, manifest.Shaders.Count);
        }

        [Fact]
        public void ReadFile_PathThatCannotBeOpenedAsAFile_ThrowsManifestException()
        {
            using var root = new TempAssetRoot();

            Assert.Throws<ShaderManifestException>(() => ShaderManifestReader.ReadFile(root.Path));
        }
    }

    public class ShaderArchiveEntryMatchingTests
    {
        private static readonly byte[] Fxp = FxpBuilder.Group(1, 0, (7, new byte[44], FxpBuilder.TinyDxbc(ShaderStage.Vertex)));

        [Fact]
        public void ReadFxp_TwoEntriesWithTheShaderPath_ThrowsArchiveException()
        {
            using var dir = new ArchiveTempDir();
            string path = dir.Write("dup.ba2", Ba2Builder.Gnrl(
            [
                new Ba2File(@"ShadersFX\Shaders011.fxp", Fxp, Compress: false),
                new Ba2File(@"Extra\ShadersFX\Shaders011.fxp", Fxp, Compress: false),
            ], version: 8));

            var ex = Assert.Throws<ShaderArchiveException>(() => ShaderArchiveReader.ReadFxp(path));

            Assert.Contains("2 entries", ex.Message);
        }

        [Fact]
        public void ReadFxp_PathMerelyEndingWithTheNameSuffix_IsNotTheShaderPackage()
        {
            using var dir = new ArchiveTempDir();
            string path = dir.Write("near.ba2", Ba2Builder.Gnrl([new Ba2File(@"oldshadersfx\Shaders011.fxp", Fxp, Compress: false)], version: 8));

            Assert.Throws<ShaderArchiveException>(() => ShaderArchiveReader.ReadFxp(path));
        }
    }
}
