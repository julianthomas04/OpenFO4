using System.Text;
using AssetConverter.ShaderTranslation.Installation;
using Xunit;

namespace InstallerTests.ShaderTranslation
{
    public sealed class ShaderFileWriterTests
    {
        private static CancellationToken Ct => TestContext.Current.CancellationToken;

        private static List<PendingShaderFile> SyntheticShaders(string directory, int count) =>
            Enumerable.Range(0, count)
                .Select(i => new PendingShaderFile(
                    Path.Combine(directory, $"shader_{i:D4}.wgsl"),
                    Encoding.UTF8.GetBytes($"// shader {i}\n" + new string((char)('a' + i % 26), 50 + i * 7)),
                    $"shader 'shader_{i:D4}'"))
                .ToList();

        [Fact]
        public void WriteAll_ParallelWrite_ProducesTheSameFilesAsASequentialWrite()
        {
            using var root = new TempAssetRoot();
            string sequentialDir = Directory.CreateDirectory(Path.Combine(root.Path, "sequential")).FullName;
            string parallelDir = Directory.CreateDirectory(Path.Combine(root.Path, "parallel")).FullName;
            foreach (var file in SyntheticShaders(sequentialDir, 300))
                File.WriteAllBytes(file.Path, file.Content);

            ShaderFileWriter.WriteAll(SyntheticShaders(parallelDir, 300), parallelism: 8, ct: Ct);

            var expected = Directory.GetFiles(sequentialDir).Select(Path.GetFileName).Order().ToList();
            var actual = Directory.GetFiles(parallelDir).Select(Path.GetFileName).Order().ToList();
            Assert.Equal(expected, actual);
            foreach (string? name in expected)
                Assert.Equal(File.ReadAllBytes(Path.Combine(sequentialDir, name!)), File.ReadAllBytes(Path.Combine(parallelDir, name!)));
        }

        [Fact]
        public void WriteAll_ExistingFile_IsReplaced()
        {
            using var root = new TempAssetRoot();
            var file = SyntheticShaders(root.Path, 1).Single();
            File.WriteAllText(file.Path, new string('x', 10_000));

            ShaderFileWriter.WriteAll([file], ct: Ct);

            Assert.Equal(file.Content, File.ReadAllBytes(file.Path));
        }

        [Fact]
        public void WriteAll_EmptyList_WritesNothing()
        {
            using var root = new TempAssetRoot();

            ShaderFileWriter.WriteAll([], ct: Ct);

            Assert.Empty(Directory.GetFiles(root.Path));
        }

        [Fact]
        public void WriteAll_FailingFiles_ThrowsTheFirstFailureInListOrderNamingItsShader()
        {
            using var root = new TempAssetRoot();
            var files = SyntheticShaders(root.Path, 40);
            files[7] = files[7] with { Path = Path.Combine(root.Path, "missing", "a.wgsl"), Label = "shader 'first_bad'" };
            files[31] = files[31] with { Path = Path.Combine(root.Path, "missing", "b.wgsl"), Label = "shader 'second_bad'" };

            var ex = Assert.Throws<ShaderTranslateStepException>(() => ShaderFileWriter.WriteAll(files, parallelism: 1, ct: Ct));

            Assert.StartsWith("shader 'first_bad': unexpected DirectoryNotFoundException", ex.Message);
            Assert.IsType<DirectoryNotFoundException>(ex.InnerException);
        }

        [Fact]
        public void WriteAll_FailingFilesWrittenConcurrently_StillThrowsTheFirstFailureInListOrder()
        {
            using var root = new TempAssetRoot();
            for (int attempt = 0; attempt < 20; attempt++)
            {
                var files = SyntheticShaders(root.Path, 60);
                files[5] = files[5] with { Path = Path.Combine(root.Path, "missing", "a.wgsl"), Label = "shader 'first_bad'" };
                files[50] = files[50] with { Path = Path.Combine(root.Path, "missing", "b.wgsl"), Label = "shader 'second_bad'" };

                var ex = Assert.Throws<ShaderTranslateStepException>(() => ShaderFileWriter.WriteAll(files, parallelism: 16, ct: Ct));

                Assert.StartsWith("shader 'first_bad'", ex.Message);
            }
        }

        [Fact]
        public void WriteAll_Cancelled_ThrowsCancellationAndWritesNothing()
        {
            using var root = new TempAssetRoot();
            using var cts = new CancellationTokenSource();
            cts.Cancel();

            Assert.ThrowsAny<OperationCanceledException>(() => ShaderFileWriter.WriteAll(SyntheticShaders(root.Path, 20), ct: cts.Token));

            Assert.Empty(Directory.GetFiles(root.Path));
        }
    }
}
