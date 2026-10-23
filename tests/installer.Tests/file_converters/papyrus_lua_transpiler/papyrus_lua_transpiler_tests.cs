using AssetConverter.PapyrusLua;
using AssetConverter.PapyrusLua.Lua;
using AssetConverter.PapyrusLua.Pex;
using InstallerTests.Support;
using Xunit;
using static InstallerTests.Support.PexModel;

namespace InstallerTests
{
    public class PapyrusLuaTranspilerTests : IDisposable
    {
        private readonly string _root = Directory.CreateTempSubdirectory("installer-tests-pex-").FullName;

        public void Dispose()
        {
            Directory.Delete(_root, recursive: true);
            GC.SuppressFinalize(this);
        }

        [Fact]
        public void ReadThenEmit_PexBytesOfAFunction_ProduceLuaThatComputesTheSameResult()
        {
            var function = Function(
                "Triple", [Op(OpCode.IMul, Id("r"), Id("x"), Int(3)), Op(OpCode.Return, Id("r"))],
                [("x", "Int")], [("r", "Int")], "Int", flags: 0x01);
            byte[] bytes = PexBytes.Write(File(Script("Maths", function)));
            using var reader = new BinaryReader(new MemoryStream(bytes));

            string lua = LuaEmitter.Emit(PexReader.Read(reader, "maths.pex"));

            Assert.Equal(21, LuaRunner.LoadSource(lua).CallGlobal("Triple", 7).Number);
        }

        [Fact]
        public void ConvertToLuaBatched_MixedValidAndCorruptFiles_WritesMirroredLuaAndLogsTheFailure()
        {
            string source = Path.Combine(_root, "src");
            string destination = Path.Combine(_root, "out");
            string log = Path.Combine(_root, "convert.log");
            WritePex(Path.Combine(source, "Top.pex"), Script("Top"));
            WritePex(Path.Combine(source, "nested", "Deep.pex"), Script("Deep"));
            Directory.CreateDirectory(source);
            System.IO.File.WriteAllBytes(Path.Combine(source, "Broken.pex"), [1, 2, 3, 4]);

            PapyrusLuaTranspiler.ConvertToLuaBatched(source, destination, log);

            Assert.Contains("-- Script: Top", System.IO.File.ReadAllText(Path.Combine(destination, "Top.lua")));
            Assert.Contains("-- Script: Deep", System.IO.File.ReadAllText(Path.Combine(destination, "nested", "Deep.lua")));
            Assert.False(System.IO.File.Exists(Path.Combine(destination, "Broken.lua")));
            string logText = System.IO.File.ReadAllText(log);
            Assert.Contains("ok:      2", logText);
            Assert.Contains("fail:    1", logText);
            Assert.Contains("Broken.pex\t", logText);
        }

        [Fact]
        public void ParseAllPex_DirectoryWithOneBadFile_ReportsCountsInTheLog()
        {
            string source = Path.Combine(_root, "parse");
            string log = Path.Combine(_root, "parse.log");
            WritePex(Path.Combine(source, "Good.pex"), Script("Good"));
            System.IO.File.WriteAllBytes(Path.Combine(source, "Bad.pex"), new byte[8]);

            PapyrusLuaTranspiler.ParseAllPex(source, log);

            string logText = System.IO.File.ReadAllText(log);
            Assert.Contains("files:  2", logText);
            Assert.Contains("ok:     1", logText);
            Assert.Contains("fail:   1", logText);
            Assert.Contains("bad magic", logText);
        }

        private static void WritePex(string path, PexObject script)
        {
            Directory.CreateDirectory(Path.GetDirectoryName(path)!);
            System.IO.File.WriteAllBytes(path, PexBytes.Write(File(script)));
        }
    }
}
