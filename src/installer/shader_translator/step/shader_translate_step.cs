using System.Security.Cryptography;
using System.Text;
using System.Text.Json;
using AssetConverter.Progress;
using AssetConverter.ShaderTranslation.Archive;
using AssetConverter.ShaderTranslation.Dxbc;
using AssetConverter.ShaderTranslation.Wgsl;

namespace AssetConverter.ShaderTranslation.Installation
{
    /// <summary>
    /// Installer step <c>shader-translate</c>: translates the shaders listed in the checked-in manifest from the game's shader archive
    /// into <c>&lt;asset_root&gt;/shaders</c>, a game asset like meshes and textures. The folder is replaced atomically; any failure leaves the previous one untouched.
    /// </summary>
    public static class ShaderTranslateStep
    {
        public const string ManifestFileName = "manifest.json";

        public static string ShadersArchivePath(string fo4Location) => Path.Combine(fo4Location, "Data", "Fallout4 - Shaders.ba2");

        public static int Run(string fo4Location, string assetRoot, string manifestPath, int progressPhase = -1) =>
            Run(fo4Location, assetRoot, manifestPath, progressPhase, CancellationToken.None);

        public static int Run(string fo4Location, string assetRoot, string manifestPath, int progressPhase, CancellationToken ct)
        {
            var manifest = ShaderManifestReader.ReadFile(manifestPath);
            var fxp = ShaderArchiveReader.ReadFxp(ShadersArchivePath(fo4Location));
            return Translate(fxp, manifest, assetRoot, new DirectoryOperations(), message => Console.Error.WriteLine(message), progressPhase, ct);
        }

        /// <summary>Translates every shader of <paramref name="manifest"/> and publishes the result; returns the number of shaders written.</summary>
        public static int Translate(FxpFile fxp, ShaderManifest manifest, string assetRoot, int progressPhase = -1) =>
            Translate(fxp, manifest, assetRoot, new DirectoryOperations(), message => Console.Error.WriteLine(message), progressPhase, CancellationToken.None);

        internal static int Translate(
            FxpFile fxp, ShaderManifest manifest, string assetRoot, DirectoryOperations operations, Action<string> warn, int progressPhase,
            CancellationToken ct)
        {
            string finalDir = Path.GetFullPath(Path.Combine(assetRoot, InstallLayout.ShadersFolder));
            string stem = finalDir.TrimEnd(Path.DirectorySeparatorChar, Path.AltDirectorySeparatorChar);
            Directory.CreateDirectory(Path.GetDirectoryName(stem)!);
            var swap = new OutputDirectorySwap(operations, warn);

            using var runLock = ShaderOutputLock.Acquire(stem);
            swap.RecoverFromKilledRun(stem);

            string tempDir = OutputDirectorySwap.NewTempPath(stem);
            Directory.CreateDirectory(tempDir);
            int shaderCount;
            try
            {
                var shaders = ShaderFamilyExpander.Expand(fxp, manifest);
                InstallProgress.AddTotal(progressPhase, shaders.Count);
                var translated = new List<TranslatedShader>(shaders.Count);
                foreach (var shader in shaders)
                {
                    ct.ThrowIfCancellationRequested();
                    translated.Add(TranslateOne(fxp, shader, tempDir));
                    InstallProgress.Tick(progressPhase);
                }
                InstallProgress.SetDetail(progressPhase, "writing shader files");
                ShaderFileWriter.WriteAll([.. translated.Select(t => t.File)], ct: ct);
                WriteOutputManifest(tempDir, translated.Select(t => t.Entry).ToList());
                swap.Publish(tempDir, finalDir, stem);
                shaderCount = shaders.Count;
            }
            catch
            {
                swap.DiscardQuietly(tempDir);
                throw;
            }
            return shaderCount;
        }

        private sealed record TranslatedShaderEntry(
            string Name, string File, string Stage, int SourceBlob, string SourceSha256, IReadOnlyList<IoRegister> Inputs, IReadOnlyList<IoRegister> Outputs);

        private sealed record TranslatedShader(TranslatedShaderEntry Entry, PendingShaderFile File);

        /// <summary>Translates in memory; <see cref="ShaderFileWriter"/> writes all the files afterwards.</summary>
        private static TranslatedShader TranslateOne(FxpFile fxp, ManifestShader shader, string outputDir)
        {
            string label = $"shader '{shader.Name}' (blob {shader.Blob})";
            try
            {
                var entry = fxp.Entries.FirstOrDefault(e => e.BlobId == shader.Blob)
                    ?? throw new ShaderTranslateStepException($"{label}: the shader archive has no such blob (it has {fxp.Entries.Count}).");
                var program = Sm5Decoder.Decode(entry.Dxbc);
                if (program.Stage != shader.Stage)
                    throw new ShaderTranslateStepException($"{label}: manifest says {shader.Stage} but the blob is a {program.Stage} shader.");
                var emitted = WgslTranslator.Emit(shader.Name, program, BuildBindingMap(shader), new WgslEmitOptions(shader.NanPolicy));

                string file = $"{shader.Name}.wgsl";
                var pending = new PendingShaderFile(Path.Combine(outputDir, file), new UTF8Encoding(false).GetBytes(emitted.Wgsl), label);
                var shaderEntry = new TranslatedShaderEntry(shader.Name, file, StageName(shader.Stage), shader.Blob, Convert.ToHexString(SHA256.HashData(entry.Dxbc.Span)).ToLowerInvariant(), emitted.Inputs, emitted.Outputs);
                return new TranslatedShader(shaderEntry, pending);
            }
            catch (ShaderTranslateStepException)
            {
                throw;
            }
            catch (ShaderTranslationException ex)
            {
                throw new ShaderTranslateStepException($"{label}: {ex.Message}", ex);
            }
            catch (Exception ex) when (ex is not OutOfMemoryException)
            {
                throw new ShaderTranslateStepException($"{label}: unexpected {ex.GetType().Name}: {ex.Message}", ex);
            }
        }

        private static ShaderBindingMap BuildBindingMap(ManifestShader shader)
        {
            var map = new ShaderBindingMap();
            foreach (var b in shader.Bindings)
            {
                var table = b.Kind switch
                {
                    RegisterKind.ConstantBuffer => map.ConstantBuffers,
                    RegisterKind.Resource => map.Resources,
                    RegisterKind.Sampler => map.Samplers,
                    _ => map.UnorderedAccess,
                };
                table[b.Register] = new ResourceBinding(b.Name, b.Group, b.Binding, b.External, b.TextureOverride, b.UavFormat);
            }
            return map;
        }

        private static string StageName(ShaderStage stage) => stage switch
        {
            ShaderStage.Vertex => "vertex",
            ShaderStage.Pixel => "fragment",
            ShaderStage.Compute => "compute",
            _ => throw new ArgumentOutOfRangeException(nameof(stage), stage, "Only vertex, fragment and compute shaders are translated."),
        };

        private static void WriteOutputManifest(string dir, List<TranslatedShaderEntry> entries)
        {
            using var buffer = new MemoryStream();
            using (var writer = new Utf8JsonWriter(buffer, new JsonWriterOptions { Indented = true }))
            {
                writer.WriteStartObject();
                writer.WriteNumber("translator_version", ShaderTranslatorVersion.Value);
                writer.WriteStartArray("shaders");
                foreach (var e in entries.OrderBy(e => e.Name, StringComparer.Ordinal))
                {
                    writer.WriteStartObject();
                    writer.WriteString("name", e.Name);
                    writer.WriteString("file", e.File);
                    writer.WriteString("stage", e.Stage);
                    writer.WriteNumber("source_blob", e.SourceBlob);
                    writer.WriteString("source_sha256", e.SourceSha256);
                    WriteInterface(writer, "inputs", e.Inputs);
                    WriteInterface(writer, "outputs", e.Outputs);
                    writer.WriteEndObject();
                }
                writer.WriteEndArray();
                writer.WriteEndObject();
            }
            ShaderFileWriter.WriteDurably(Path.Combine(dir, ManifestFileName), buffer.ToArray());
        }

        /// <summary>One element per signature element; a register shared by several elements appears once per element with its own mask.</summary>
        private static void WriteInterface(Utf8JsonWriter writer, string property, IReadOnlyList<IoRegister> registers)
        {
            writer.WriteStartArray(property);
            foreach (var register in registers)
            {
                var elements = register.Elements.Count > 0 ? register.Elements : [new IoElement("", 0, register.Mask)];
                foreach (var element in elements)
                {
                    writer.WriteStartObject();
                    writer.WriteNumber("register", register.Register);
                    writer.WriteString("semantic", element.SemanticName);
                    writer.WriteNumber("index", element.SemanticIndex);
                    writer.WriteNumber("mask", element.Mask);
                    writer.WriteNumber("system_value", register.SystemValue);
                    writer.WriteEndObject();
                }
            }
            writer.WriteEndArray();
        }
    }
}
