namespace InstallerTests.Support
{
    /// <summary>A material file on disk in a unique temp directory, deleted on dispose (the code under test takes a FileStream).</summary>
    internal sealed class TempMaterialFile : IDisposable
    {
        private readonly string _directory;
        private readonly string _path;

        private TempMaterialFile(string directory, string path)
        {
            _directory = directory;
            _path = path;
        }

        public static TempMaterialFile Write(byte[] bytes)
        {
            string directory = Directory.CreateTempSubdirectory("installer-tests-").FullName;
            string path = Path.Combine(directory, "material.bin");
            File.WriteAllBytes(path, bytes);
            return new TempMaterialFile(directory, path);
        }

        public T Use<T>(Func<FileStream, T> action)
        {
            using var stream = new FileStream(_path, FileMode.Open, FileAccess.Read);
            return action(stream);
        }

        public void Dispose() => Directory.Delete(_directory, recursive: true);
    }
}
