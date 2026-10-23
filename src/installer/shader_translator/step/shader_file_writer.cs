namespace AssetConverter.ShaderTranslation.Installation
{
    /// <summary>A shader file held in memory until the writer persists it; <see cref="Label"/> names the shader in error messages.</summary>
    internal sealed record PendingShaderFile(string Path, byte[] Content, string Label);

    /// <summary>
    /// Persists translated shaders durably. Every file is flushed to the device so a power loss cannot publish a shaders folder whose
    /// files are empty; the flushes dominate the step while other lanes write to the same volume, so they overlap across files.
    /// </summary>
    internal static class ShaderFileWriter
    {
        /// <summary>Writes every file and returns when all of them are on the device; a failure stops the rest and the first failing file in list order is thrown.</summary>
        public static void WriteAll(IReadOnlyList<PendingShaderFile> files, int parallelism = IoParallelism.MaxThreads, CancellationToken ct = default)
        {
            var failures = ParallelFailures.Collect(
                files.Count, parallelism, index => WriteDurably(files[index].Path, files[index].Content), stopAtFirstFailure: true, ct: ct);
            if (failures.Count == 0) return;

            var (first, cause) = failures[0];
            throw new ShaderTranslateStepException($"{files[first].Label}: unexpected {cause.GetType().Name}: {cause.Message}", cause);
        }

        public static void WriteDurably(string path, byte[] content)
        {
            using var stream = new FileStream(path, FileMode.Create, FileAccess.Write, FileShare.None);
            stream.Write(content);
            stream.Flush(flushToDisk: true);
        }
    }
}
