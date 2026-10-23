using System.Text.Json;
using AssetConverter.ShaderTranslation;
using AssetConverter.ShaderTranslation.Archive;
using AssetConverter.ShaderTranslation.Dxbc;
using AssetConverter.ShaderTranslation.Installation;
using AssetConverter.ShaderTranslation.Wgsl;
using InstallerTests.Support;
using Xunit;
using static InstallerTests.Support.Sm5Assembler;

namespace InstallerTests.ShaderTranslation
{
    public class ShaderFamilyTests
    {
        private static readonly uint[] AllComponents = Swz(X, Y, Z, W);

        private static byte[] CopyInputToOutputVertex(params (string Name, uint Index, uint Register, byte Mask)[] inputs) =>
            new Sm5Assembler(ShaderStage.Vertex).InputDeclaration(1, 0xF).OutputDeclaration(0, 0xF)
                .Instruction(Opcode.Mov, 0, Dst(OperandType.Output, 0, 0xF), Src(OperandType.Input, 1, AllComponents))
                .Instruction(Opcode.Ret, 0)
                .Chunk("ISGN", Signature(inputs.Select(i => (i.Name, i.Index, 0u, i.Register, i.Mask)).ToArray()))
                .Chunk("OSGN", Signature(("TEXCOORD", 3u, 0u, 0u, (byte)0xF)))
                .BuildDxbc();

        private static byte[] PixelCopyingCb0() =>
            new Sm5Assembler(ShaderStage.Pixel).ConstantBufferDeclaration(0, 1).OutputDeclaration(0, 0xF).Temps(1)
                .Instruction(Opcode.Mov, 0, Dst(OperandType.Output, 0, 0xF), ConstantBuffer(0, 0, AllComponents))
                .Instruction(Opcode.Ret, 0)
                .Chunk("OSGN", Signature(("SV_Target", 0u, 0u, 0u, (byte)0xF)))
                .BuildDxbc();

        private static FxpFile Archive(uint vertexCount, uint pixelCount, params (uint Descriptor, byte[] Dxbc)[] entries) =>
            new([new FxpGroupHeader(vertexCount, 0, 0, pixelCount, 0)],
                entries.Select((e, i) => new FxpEntry(i, 0, e.Descriptor, ReadOnlyMemory<byte>.Empty, e.Dxbc)).ToList());

        private static readonly (string Name, uint Index, uint Register, byte Mask)[] SharedRegisterInputs =
            [("TEXCOORD", 0, 1, 0x3), ("TEXCOORD", 1, 1, 0xC)];

        private const string FamilyJson = """
            {"shaders":[],"families":[{"family":"fx","fxp_group":0,"name":"fx_{stage}_{descriptor:x8}","vertex_count":2,"pixel_count":1,
              "bindings":{"vertex":[],"fragment":[{"register":"cb0","name":"fx_frame.ps_cb0","external":true}]}}]}
            """;

        private static ShaderManifest FamilyManifest(string json = FamilyJson) => ShaderManifestReader.Parse(json);

        private static FxpFile TwoVertexOnePixelArchive() => Archive(2, 1,
            (0x15, CopyInputToOutputVertex(SharedRegisterInputs)),
            (0x35, CopyInputToOutputVertex(SharedRegisterInputs)),
            (0x15, PixelCopyingCb0()));

        private static JsonDocument TranslateToManifest(FxpFile fxp, ShaderManifest manifest, TempAssetRoot root)
        {
            ShaderTranslateStep.Translate(fxp, manifest, root.Path);
            return JsonDocument.Parse(File.ReadAllText(Path.Combine(root.Shaders, "manifest.json")));
        }

        [Fact]
        public void Translate_Family_WritesOneShaderPerBlobNamedByStageAndDescriptor()
        {
            using var root = new TempAssetRoot();

            int count = ShaderTranslateStep.Translate(TwoVertexOnePixelArchive(), FamilyManifest(), root.Path);

            Assert.Equal(3, count);
            Assert.True(File.Exists(Path.Combine(root.Shaders, "fx_vs_00000015.wgsl")));
            Assert.True(File.Exists(Path.Combine(root.Shaders, "fx_vs_00000035.wgsl")));
            Assert.True(File.Exists(Path.Combine(root.Shaders, "fx_ps_00000015.wgsl")));
        }

        [Fact]
        public void Translate_Family_UsesTheStageSpecificBindings()
        {
            using var root = new TempAssetRoot();

            ShaderTranslateStep.Translate(TwoVertexOnePixelArchive(), FamilyManifest(), root.Path);

            Assert.Contains("fx_frame.ps_cb0[0]", File.ReadAllText(Path.Combine(root.Shaders, "fx_ps_00000015.wgsl")));
        }

        [Fact]
        public void Translate_Family_RecordsBlobAndStageInTheOutputManifest()
        {
            using var root = new TempAssetRoot();

            using var doc = TranslateToManifest(TwoVertexOnePixelArchive(), FamilyManifest(), root);

            var shaders = doc.RootElement.GetProperty("shaders").EnumerateArray()
                .ToDictionary(s => s.GetProperty("name").GetString()!, s => (s.GetProperty("stage").GetString(), s.GetProperty("source_blob").GetInt32()));
            Assert.Equal(("vertex", 0), shaders["fx_vs_00000015"]);
            Assert.Equal(("vertex", 1), shaders["fx_vs_00000035"]);
            Assert.Equal(("fragment", 2), shaders["fx_ps_00000015"]);
        }

        [Fact]
        public void Translate_FamilyAlongsideExplicitShaders_TranslatesBoth()
        {
            using var root = new TempAssetRoot();
            string json = FamilyJson.Replace("\"shaders\":[]", """
                "shaders":[{"name":"extra_vs","blob":0,"stage":"vertex"}]
                """);

            int count = ShaderTranslateStep.Translate(TwoVertexOnePixelArchive(), FamilyManifest(json), root.Path);

            Assert.Equal(4, count);
            Assert.True(File.Exists(Path.Combine(root.Shaders, "extra_vs.wgsl")));
        }

        [Fact]
        public void Translate_Family_SameDescriptorInBothStagesIsAllowed()
        {
            using var root = new TempAssetRoot();

            // fx_vs_00000015 and fx_ps_00000015 share a descriptor; the stage keeps the names apart.
            ShaderTranslateStep.Translate(TwoVertexOnePixelArchive(), FamilyManifest(), root.Path);

            Assert.True(File.Exists(Path.Combine(root.Shaders, "fx_vs_00000015.wgsl")));
            Assert.True(File.Exists(Path.Combine(root.Shaders, "fx_ps_00000015.wgsl")));
        }

        [Fact]
        public void Translate_Family_DuplicateDescriptorWithinAStage_ThrowsNamingBothBlobsAndLeavesNoOutput()
        {
            using var root = new TempAssetRoot();
            var fxp = Archive(2, 1,
                (0x15, CopyInputToOutputVertex(SharedRegisterInputs)),
                (0x15, CopyInputToOutputVertex(SharedRegisterInputs)),
                (0x15, PixelCopyingCb0()));

            var ex = Assert.Throws<ShaderTranslateStepException>(() => ShaderTranslateStep.Translate(fxp, FamilyManifest(), root.Path));

            Assert.Contains("0x00000015", ex.Message);
            Assert.Contains("blobs 0 and 1", ex.Message);
            Assert.False(Directory.Exists(root.Shaders));
        }

        [Theory]
        [InlineData(3, 1)]
        [InlineData(2, 2)]
        public void Translate_Family_HeaderCountsDifferingFromTheManifest_Throw(uint vertexCount, uint pixelCount)
        {
            using var root = new TempAssetRoot();
            var fxp = new FxpFile([new FxpGroupHeader(vertexCount, 0, 0, pixelCount, 0)], TwoVertexOnePixelArchive().Entries);

            var ex = Assert.Throws<ShaderTranslateStepException>(() => ShaderTranslateStep.Translate(fxp, FamilyManifest(), root.Path));

            Assert.Contains("different game build", ex.Message);
        }

        [Fact]
        public void Translate_Family_EntryCountDifferingFromTheHeader_Throws()
        {
            using var root = new TempAssetRoot();
            var full = TwoVertexOnePixelArchive();
            var fxp = new FxpFile(full.Groups, full.Entries.Take(2).ToList());

            var ex = Assert.Throws<ShaderTranslateStepException>(() => ShaderTranslateStep.Translate(fxp, FamilyManifest(), root.Path));

            Assert.Contains("holds 2 entries", ex.Message);
        }

        [Fact]
        public void Translate_Family_GroupMissingFromTheArchive_Throws()
        {
            using var root = new TempAssetRoot();
            string json = FamilyJson.Replace("\"fxp_group\":0", "\"fxp_group\":4");

            var ex = Assert.Throws<ShaderTranslateStepException>(() => ShaderTranslateStep.Translate(TwoVertexOnePixelArchive(), FamilyManifest(json), root.Path));

            Assert.Contains("only 1 groups", ex.Message);
        }

        [Fact]
        public void Translate_Family_BlobTranslationFailureIsAHardError()
        {
            using var root = new TempAssetRoot();
            var unsupported = new Sm5Assembler(ShaderStage.Pixel).Temps(1)
                .Instruction(Opcode.Lod, 0, Dst(OperandType.Temp, 0, 0b0001), Src(OperandType.Temp, 0, AllComponents), Resource(0, AllComponents), Sampler(0))
                .Instruction(Opcode.Ret, 0).BuildDxbc();
            var fxp = Archive(2, 1, (0x15, CopyInputToOutputVertex(SharedRegisterInputs)), (0x35, CopyInputToOutputVertex(SharedRegisterInputs)), (0x15, unsupported));

            var ex = Assert.Throws<ShaderTranslateStepException>(() => ShaderTranslateStep.Translate(fxp, FamilyManifest(), root.Path));

            Assert.Contains("fx_ps_00000015", ex.Message);
            Assert.False(Directory.Exists(root.Shaders));
        }

        [Fact]
        public void Translate_FamilyNameCollidingWithAnExplicitShader_Throws()
        {
            using var root = new TempAssetRoot();
            string json = FamilyJson.Replace("\"shaders\":[]", """
                "shaders":[{"name":"FX_VS_00000015","blob":0,"stage":"vertex"}]
                """);

            Assert.Throws<ShaderManifestException>(() => ShaderTranslateStep.Translate(TwoVertexOnePixelArchive(), FamilyManifest(json), root.Path));
        }

        [Fact]
        public void OutputManifest_IsVersion2()
        {
            using var root = new TempAssetRoot();

            using var doc = TranslateToManifest(TwoVertexOnePixelArchive(), FamilyManifest(), root);

            Assert.Equal(2u, doc.RootElement.GetProperty("translator_version").GetUInt32());
        }

        [Fact]
        public void OutputManifest_EveryEntryCarriesInputsAndOutputs()
        {
            using var root = new TempAssetRoot();

            using var doc = TranslateToManifest(TwoVertexOnePixelArchive(), FamilyManifest(), root);

            foreach (var shader in doc.RootElement.GetProperty("shaders").EnumerateArray())
            {
                Assert.Equal(["name", "file", "stage", "source_blob", "source_sha256", "inputs", "outputs"], shader.EnumerateObject().Select(p => p.Name));
                Assert.Equal(JsonValueKind.Array, shader.GetProperty("inputs").ValueKind);
                Assert.Equal(JsonValueKind.Array, shader.GetProperty("outputs").ValueKind);
            }
        }

        [Fact]
        public void OutputManifest_RegistersSharedBetweenSemantics_AreListedOncePerSemanticWithTheirOwnMask()
        {
            using var root = new TempAssetRoot();

            using var doc = TranslateToManifest(TwoVertexOnePixelArchive(), FamilyManifest(), root);

            var vertex = doc.RootElement.GetProperty("shaders").EnumerateArray().First(s => s.GetProperty("name").GetString() == "fx_vs_00000015");
            var inputs = vertex.GetProperty("inputs").EnumerateArray()
                .Select(i => (i.GetProperty("register").GetInt32(), i.GetProperty("semantic").GetString(), i.GetProperty("index").GetInt32(), i.GetProperty("mask").GetInt32(), i.GetProperty("system_value").GetInt32()))
                .ToList();
            Assert.Equal([(1, "TEXCOORD", 0, 0x3, 0), (1, "TEXCOORD", 1, 0xC, 0)], inputs);
            var output = Assert.Single(vertex.GetProperty("outputs").EnumerateArray());
            Assert.Equal((0, "TEXCOORD", 3, 0xF), (output.GetProperty("register").GetInt32(), output.GetProperty("semantic").GetString(), output.GetProperty("index").GetInt32(), output.GetProperty("mask").GetInt32()));
        }

        [Fact]
        public void OutputManifest_PixelShaderTarget_IsListedAsSystemValueSemantic()
        {
            using var root = new TempAssetRoot();

            using var doc = TranslateToManifest(TwoVertexOnePixelArchive(), FamilyManifest(), root);

            var pixel = doc.RootElement.GetProperty("shaders").EnumerateArray().First(s => s.GetProperty("name").GetString() == "fx_ps_00000015");
            var target = Assert.Single(pixel.GetProperty("outputs").EnumerateArray());
            Assert.Equal(("SV_Target", 0, 0xF), (target.GetProperty("semantic").GetString(), target.GetProperty("index").GetInt32(), target.GetProperty("mask").GetInt32()));
            Assert.Empty(pixel.GetProperty("inputs").EnumerateArray());
        }

        [Fact]
        public void OutputManifest_DeclaredRegisterWithoutSignatureElement_IsListedWithAnEmptySemantic()
        {
            using var root = new TempAssetRoot();
            var noSignature = new Sm5Assembler(ShaderStage.Vertex).InputDeclaration(1, 0x7).OutputDeclaration(0, 0xF)
                .Instruction(Opcode.Mov, 0, Dst(OperandType.Output, 0, 0xF), Src(OperandType.Input, 1, AllComponents))
                .Instruction(Opcode.Ret, 0).BuildDxbc();
            var fxp = Archive(1, 0, (0x1, noSignature));
            string json = """{"shaders":[],"families":[{"family":"fx","fxp_group":0,"name":"fx_{stage}_{descriptor:x8}","vertex_count":1,"pixel_count":0,"bindings":{"vertex":[],"fragment":[]}}]}""";

            using var doc = TranslateToManifest(fxp, FamilyManifest(json), root);

            var input = Assert.Single(doc.RootElement.GetProperty("shaders").EnumerateArray().Single().GetProperty("inputs").EnumerateArray());
            Assert.Equal(("", 0, 0x7), (input.GetProperty("semantic").GetString(), input.GetProperty("index").GetInt32(), input.GetProperty("mask").GetInt32()));
        }

        [Fact]
        public void ExpandName_SubstitutesStageAndEightDigitLowercaseHexDescriptor()
        {
            Assert.Equal("effect_ps_00a06015", ManifestFamily.ExpandName("effect_{stage}_{descriptor:x8}", "ps", 0xA06015));
        }
    }

    public class ShaderManifestFamilyReaderTests
    {
        private static string Family(string name = "fx_{stage}_{descriptor:x8}", string group = "4", string extra = "", string family = "fx", string bindings = """{"vertex":[],"fragment":[]}""") =>
            $$"""{"family":"{{family}}","fxp_group":{{group}},"name":"{{name}}","vertex_count":474,"pixel_count":968{{extra}},"bindings":{{bindings}}}""";

        private static ShaderManifest Parse(params string[] families) =>
            ShaderManifestReader.Parse($$"""{"shaders":[],"families":[{{string.Join(",", families)}}]}""");

        [Fact]
        public void Parse_Family_MapsEveryField()
        {
            var manifest = Parse(Family(extra: ",\"nan_policy\":\"d3d_strict\"", bindings: """
                {"vertex":[{"register":"cb12","name":"fx_frame.cb12","external":true}],
                 "fragment":[{"register":"t3","name":"fx_t3","external":true,"texture_override":"depth"}]}
                """));

            var family = Assert.Single(manifest.Families);
            Assert.Equal(("fx", 4, "fx_{stage}_{descriptor:x8}", 474, 968, NanPolicy.D3dStrict), (family.Family, family.FxpGroup, family.NameTemplate, family.VertexCount, family.PixelCount, family.NanPolicy));
            Assert.Equal([new ManifestBinding(RegisterKind.ConstantBuffer, 12, "fx_frame.cb12", 0, 0, true, TextureOverride.None, null)], family.VertexBindings);
            Assert.Equal([new ManifestBinding(RegisterKind.Resource, 3, "fx_t3", 0, 0, true, TextureOverride.Depth, null)], family.PixelBindings);
        }

        [Fact]
        public void Parse_NoFamilies_YieldsEmptyFamilyList()
        {
            Assert.Empty(ShaderManifestReader.Parse("""{"shaders":[]}""").Families);
        }

        [Theory]
        [InlineData("fx_{stage}")]
        [InlineData("fx_{descriptor:x8}")]
        [InlineData("fx-{stage}-{descriptor:x8}")]
        [InlineData("__fx_{stage}_{descriptor:x8}")]
        public void Parse_BadNameTemplate_Throws(string template)
        {
            Assert.Throws<ShaderManifestException>(() => Parse(Family(name: template)));
        }

        [Fact]
        public void Parse_TwoFamiliesOnTheSameGroup_Throws()
        {
            Assert.Throws<ShaderManifestException>(() => Parse(Family(), Family(family: "other")));
        }

        [Fact]
        public void Parse_UnknownFamilyField_Throws()
        {
            Assert.Throws<ShaderManifestException>(() => Parse(Family(extra: ",\"bogus\":1")));
        }

        [Fact]
        public void Parse_FamilyBindingsWithoutStageSplit_Throws()
        {
            Assert.Throws<ShaderManifestException>(() => Parse(Family(bindings: "[]")));
        }

        [Fact]
        public void Parse_NonExternalBindingWithoutSlot_StillThrows()
        {
            Assert.Throws<ShaderManifestException>(() => Parse(Family(bindings: """{"vertex":[{"register":"cb0","name":"fx_cb0"}],"fragment":[]}""")));
        }

        [Fact]
        public void CheckedInManifest_DeclaresTheEffectFamilyOfFxpGroup4WithItsHeaderCounts()
        {
            var manifest = ShaderManifestReader.Parse(RepoFiles.ReadAllText(ShaderManifestReader.RepoRelativePath));

            var family = Assert.Single(manifest.Families);
            Assert.Equal(("effect", 4, "effect_{stage}_{descriptor:x8}", 474, 968), (family.Family, family.FxpGroup, family.NameTemplate, family.VertexCount, family.PixelCount));
            Assert.All(family.VertexBindings.Concat(family.PixelBindings), b => Assert.True(b.External));
            Assert.Equal(TextureOverride.Depth, family.PixelBindings.Single(b => b is { Kind: RegisterKind.Resource, Register: 3 }).TextureOverride);
        }
    }
}
