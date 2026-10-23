using AssetConverter.InstallerUi;
using Avalonia.Platform.Storage;
using Xunit;

namespace InstallerTests.Ui
{
    public sealed class PathOpenerTests : IDisposable
    {
        private readonly string _folder = Directory.CreateTempSubdirectory("path-opener").FullName;
        private readonly FakeLauncher _launcher = new();
        private readonly List<string> _errors = [];
        private readonly PathOpener _opener;

        public PathOpenerTests()
        {
            _opener = new PathOpener(_launcher, _errors.Add);
        }

        public void Dispose() => Directory.Delete(_folder, recursive: true);

        [Fact]
        public async Task AnExistingFolderIsHandedToTheLauncher()
        {
            bool opened = await _opener.OpenFolderAsync(_folder);

            Assert.True(opened);
            Assert.Equal([_folder], _launcher.Launched.Select(Path.TrimEndingDirectorySeparator));
            Assert.Empty(_errors);
        }

        [Fact]
        public async Task AnExistingFileIsHandedToTheLauncher()
        {
            string file = Path.Combine(_folder, "install.log");
            await File.WriteAllTextAsync(file, "log", TestContext.Current.CancellationToken);

            bool opened = await _opener.OpenFileAsync(file);

            Assert.True(opened);
            Assert.Equal([file], _launcher.Launched);
            Assert.Empty(_errors);
        }

        [Fact]
        public async Task AMissingPathIsReportedAndNeverLaunched()
        {
            string missing = Path.Combine(_folder, "missing.log");

            bool opened = await _opener.OpenFileAsync(missing);

            Assert.False(opened);
            Assert.Empty(_launcher.Launched);
            Assert.Equal([$"ERROR: cannot open the file {missing}: it does not exist."], _errors);
        }

        [Fact]
        public async Task ALauncherThatDeclinesIsReported()
        {
            _launcher.Result = false;

            bool opened = await _opener.OpenFolderAsync(_folder);

            Assert.False(opened);
            Assert.Equal([$"ERROR: the system could not open the folder {_folder}."], _errors);
        }

        [Fact]
        public async Task ALauncherThatThrowsIsReportedWithItsException()
        {
            _launcher.Failure = new InvalidOperationException("no handler registered");

            bool opened = await _opener.OpenFolderAsync(_folder);

            Assert.False(opened);
            string error = Assert.Single(_errors);
            Assert.StartsWith($"ERROR: opening the folder {_folder} failed:", error, StringComparison.Ordinal);
            Assert.Contains("no handler registered", error, StringComparison.Ordinal);
        }

        [Fact]
        public async Task AnUnexpectedExceptionIsNotSwallowed()
        {
            _launcher.Failure = new ArgumentException("defect");

            await Assert.ThrowsAsync<ArgumentException>(() => _opener.OpenFolderAsync(_folder));
        }

        private sealed class FakeLauncher : ILauncher
        {
            public List<string> Launched { get; } = [];

            public bool Result { get; set; } = true;

            public Exception? Failure { get; set; }

            public Task<bool> LaunchUriAsync(Uri uri) => throw new NotSupportedException();

            public Task<bool> LaunchFileAsync(IStorageItem storageItem)
            {
                if (Failure != null) throw Failure;
                Launched.Add(storageItem.Path.LocalPath);
                return Task.FromResult(Result);
            }
        }
    }
}
