using AssetConverter.InstallerUi;
using Xunit;

namespace InstallerTests.Ui
{
    public sealed class SetupViewModelTests : IDisposable
    {
        private readonly TempDirectory _root = new();

        public void Dispose() => _root.Dispose();

        private string MakeGameFolder()
        {
            string game = Path.Combine(_root.Path, "Fallout 4");
            Directory.CreateDirectory(Path.Combine(game, "Data"));
            File.WriteAllBytes(Path.Combine(game, "Fallout4.exe"), []);
            File.WriteAllBytes(Path.Combine(game, "Data", "Fallout4.esm"), []);
            return game;
        }

        private string ChosenFolder => Path.Combine(_root.Path, "Games");

        private string InstallRoot => Path.Combine(ChosenFolder, "OpenFO4");

        private sealed class FakePicker(string? picked) : IFolderPicker
        {
            public List<(string Title, string? StartFolder)> Requests { get; } = [];

            public Task<string?> PickFolderAsync(string title, string? startFolder)
            {
                Requests.Add((title, startFolder));
                return Task.FromResult(picked);
            }
        }

        [Fact]
        public void ANewSetupHasNothingChosenAndCannotStart()
        {
            var setup = new SetupViewModel();

            Assert.Equal("", setup.GameFolder);
            Assert.Equal("", setup.InstallFolder);
            Assert.False(setup.KeepStaging);
            Assert.Equal("Choose the Fallout 4 folder.", setup.GameFolderError);
            Assert.Equal("Choose where to install.", setup.InstallFolderError);
            Assert.False(setup.CanStart);
            Assert.False(setup.Completed.IsCompleted);
        }

        [Fact]
        public void TheGameFolderAndTheKeepOptionCanBePrefilled()
        {
            var setup = new SetupViewModel(MakeGameFolder(), keepStaging: true);

            Assert.Equal("", setup.GameFolderError);
            Assert.True(setup.KeepStaging);
            Assert.Equal("", setup.InstallFolder);
        }

        [Fact]
        public void StartNeedsBothFoldersValid()
        {
            var setup = new SetupViewModel(MakeGameFolder());
            Assert.False(setup.CanStart);

            setup.InstallFolder = ChosenFolder;
            Assert.True(setup.CanStart);

            setup.GameFolder = Path.Combine(_root.Path, "missing");
            Assert.False(setup.CanStart);
            Assert.Equal("This folder does not exist.", setup.GameFolderError);
        }

        [Fact]
        public void StartDoesNothingWhileAFolderIsInvalid()
        {
            var setup = new SetupViewModel(MakeGameFolder());

            setup.Start();

            Assert.False(setup.Completed.IsCompleted);
        }

        [Fact]
        public async Task StartYieldsTheFullPathsAndTheOptionAndLocksTheSetup()
        {
            string game = MakeGameFolder();
            var setup = new SetupViewModel($"\"{game}\"");
            setup.InstallFolder = ChosenFolder + Path.DirectorySeparatorChar;
            setup.KeepStaging = true;

            setup.Start();

            var choice = await setup.Completed;
            Assert.Equal(new SetupChoice(game, InstallRoot, KeepStaging: true), choice);
            Assert.False(setup.CanEdit);
            Assert.False(setup.CanStart);
        }

        [Fact]
        public void StartCreatesTheInstallFolderAndClaimsItAsAnInstall()
        {
            var setup = new SetupViewModel(MakeGameFolder());
            setup.InstallFolder = ChosenFolder;

            setup.Start();

            Assert.True(setup.HasStarted);
            Assert.True(File.Exists(Path.Combine(InstallRoot, AssetConverter.InstallLayout.OwnershipMarkerFileName)));
        }

        [Fact]
        public void ARunningGameInTheInstallFolderKeepsTheSetupOpenAndAsksToCloseIt()
        {
            string exe = Path.Combine(InstallRoot, "openfo4.exe");
            new AssetConverter.Pipeline.InstallLocations("", InstallRoot).CreateOutputDirectories();
            File.WriteAllText(exe, "game");
            using var running = new FileStream(exe, FileMode.Open, FileAccess.Read, FileShare.Read);
            var setup = new SetupViewModel(MakeGameFolder());
            setup.InstallFolder = ChosenFolder;

            setup.Start();

            Assert.False(setup.HasStarted);
            Assert.True(setup.CanEdit);
            Assert.StartsWith("Close OpenFO4 before reinstalling", setup.StartError, StringComparison.Ordinal);
        }

        [Fact]
        public void AnInstallFolderThatCannotBeCreatedKeepsTheSetupOpenAndSaysWhy()
        {
            string blocker = Path.Combine(_root.Path, "blocker");
            File.WriteAllText(blocker, "a file");
            var setup = new SetupViewModel(MakeGameFolder());
            setup.InstallFolder = Path.Combine(blocker, "install");

            setup.Start();

            Assert.False(setup.HasStarted);
            Assert.False(setup.Completed.IsCompleted);
            Assert.True(setup.CanEdit);
            Assert.StartsWith("Cannot create the install folder", setup.StartError, StringComparison.Ordinal);
        }

        [Fact]
        public void TheStartErrorGoesAwayWithTheNextEditAndAnotherFolderCanBeStarted()
        {
            string blocker = Path.Combine(_root.Path, "blocker");
            File.WriteAllText(blocker, "a file");
            var setup = new SetupViewModel(MakeGameFolder());
            setup.InstallFolder = Path.Combine(blocker, "install");
            setup.Start();

            setup.InstallFolder = ChosenFolder;
            Assert.Equal("", setup.StartError);
            setup.Start();

            Assert.True(setup.HasStarted);
        }

        [Fact]
        public void TheFoldersAreCheckedAgainWhenInstallIsPressed()
        {
            var setup = new SetupViewModel(MakeGameFolder());
            setup.InstallFolder = ChosenFolder;
            Assert.Equal("", setup.InstallFolderError);
            Directory.CreateDirectory(InstallRoot);
            File.WriteAllText(Path.Combine(InstallRoot, "photos.jpg"), "x");

            setup.Start();

            Assert.False(setup.HasStarted);
            Assert.Equal($"{InstallRoot} already contains other files; choose an empty folder or a previous OpenFO4 install.", setup.InstallFolderError);
            Assert.False(File.Exists(Path.Combine(InstallRoot, AssetConverter.InstallLayout.OwnershipMarkerFileName)));
        }

        [Fact]
        public void TheValidationIsComputedOncePerEditNotOncePerRead()
        {
            Directory.CreateDirectory(InstallRoot);
            File.WriteAllText(Path.Combine(InstallRoot, "photos.jpg"), "x");
            var setup = new SetupViewModel(MakeGameFolder());
            setup.InstallFolder = ChosenFolder;
            Assert.NotEqual("", setup.InstallFolderError);

            File.Delete(Path.Combine(InstallRoot, "photos.jpg"));

            Assert.NotEqual("", setup.InstallFolderError);
            setup.InstallFolder = ChosenFolder + Path.DirectorySeparatorChar;
            Assert.Equal("", setup.InstallFolderError);
        }

        [Fact]
        public async Task CancelEndsTheSetupWithoutAChoice()
        {
            var setup = new SetupViewModel(MakeGameFolder());
            setup.InstallFolder = ChosenFolder;

            setup.Cancel();
            setup.Start();

            Assert.Null(await setup.Completed);
        }

        [Fact]
        public async Task CancelAfterStartDoesNotTakeTheChoiceBack()
        {
            var setup = new SetupViewModel(MakeGameFolder());
            setup.InstallFolder = ChosenFolder;
            setup.Start();

            setup.Cancel();

            Assert.NotNull(await setup.Completed);
        }

        [Fact]
        public void AnInstallFolderInsideTheGameFolderBecomesInvalidWhenTheGameFolderIsChosen()
        {
            string game = MakeGameFolder();
            var setup = new SetupViewModel();
            setup.InstallFolder = Path.Combine(game, "OpenFO4");
            Assert.Equal("", setup.InstallFolderError);

            setup.GameFolder = game;

            Assert.Equal($"{Path.Combine(game, "OpenFO4", "OpenFO4")} is inside the Fallout 4 folder {game}; choose a folder outside it.", setup.InstallFolderError);
            Assert.False(setup.CanStart);
        }

        [Fact]
        public void ChangingAFolderRaisesTheChangesOfItsValidation()
        {
            var setup = new SetupViewModel();
            var changed = new List<string?>();
            setup.PropertyChanged += (_, e) => changed.Add(e.PropertyName);

            setup.GameFolder = MakeGameFolder();

            Assert.Contains(nameof(SetupViewModel.GameFolder), changed);
            Assert.Contains(nameof(SetupViewModel.GameFolderError), changed);
            Assert.Contains(nameof(SetupViewModel.InstallFolderError), changed);
            Assert.Contains(nameof(SetupViewModel.CanStart), changed);
        }

        [Fact]
        public async Task BrowsingFillsTheFolderWithThePickedOneAndStartsAtTheCurrentOne()
        {
            var setup = new SetupViewModel();
            setup.InstallFolder = ChosenFolder;
            var picker = new FakePicker(@"D:\Games\Fallout 4");

            await setup.BrowseGameFolderAsync(picker);
            await setup.BrowseInstallFolderAsync(picker);

            Assert.Equal(@"D:\Games\Fallout 4", setup.GameFolder);
            Assert.Equal(@"D:\Games\Fallout 4", setup.InstallFolder);
            Assert.Equal([("Select the Fallout 4 folder", (string?)null), ("Select the folder to install OpenFO4 in", ChosenFolder)], picker.Requests);
        }

        [Fact]
        public async Task DismissingTheFolderDialogChangesNothing()
        {
            var setup = new SetupViewModel(MakeGameFolder());

            await setup.BrowseGameFolderAsync(new FakePicker(null));

            Assert.Equal(MakeGameFolder(), setup.GameFolder);
        }

        private string MakeDocumentsFolder() => Directory.CreateDirectory(Path.Combine(_root.Path, "Documents", "My Games", "Fallout4")).FullName;

        [Fact]
        public void ALaunchCreatedIniMissingBlocksStartWithTheStartFirstMessage()
        {
            string game = MakeGameFolder();
            string documents = MakeDocumentsFolder();
            File.WriteAllText(Path.Combine(game, "Fallout4_Default.ini"), "");
            var setup = new SetupViewModel(game, settingsFolder: documents) { InstallFolder = ChosenFolder };

            Assert.False(setup.CanStart);
            Assert.Equal("", setup.GameFolderError);
            Assert.StartsWith("Fallout 4 has not been started yet.", setup.GameIniError);
            Assert.EndsWith($"Missing: {Path.Combine(documents, "Fallout4Prefs.ini")}.", setup.GameIniError);
            setup.Start();
            Assert.False(setup.Completed.IsCompleted);
        }

        [Fact]
        public void RevalidatingPicksUpIniFilesTheGameWroteMeanwhile()
        {
            string game = MakeGameFolder();
            string documents = MakeDocumentsFolder();
            File.WriteAllText(Path.Combine(game, "Fallout4_Default.ini"), "");
            var setup = new SetupViewModel(game, settingsFolder: documents) { InstallFolder = ChosenFolder };
            var changed = new List<string?>();
            setup.PropertyChanged += (_, e) => changed.Add(e.PropertyName);

            File.WriteAllText(Path.Combine(documents, "Fallout4Prefs.ini"), "");
            setup.Revalidate();

            Assert.Equal("", setup.GameIniError);
            Assert.True(setup.CanStart);
            Assert.Contains(nameof(SetupViewModel.GameIniError), changed);
        }

        [Fact]
        public void AMissingShippedIniSaysTheInstallIsIncomplete()
        {
            string game = MakeGameFolder();
            string documents = MakeDocumentsFolder();
            File.WriteAllText(Path.Combine(documents, "Fallout4Prefs.ini"), "");
            var setup = new SetupViewModel(game, settingsFolder: documents);

            Assert.StartsWith("The Fallout 4 installation looks incomplete.", setup.GameIniError);
            Assert.Contains("Verify the game files in Steam", setup.GameIniError);
        }

        [Fact]
        public async Task ChoosingAnotherSettingsFolderMakesTheCheckPassAndTheChoiceCarriesIt()
        {
            string game = MakeGameFolder();
            File.WriteAllText(Path.Combine(game, "Fallout4_Default.ini"), "");
            string elsewhere = Directory.CreateDirectory(Path.Combine(_root.Path, "Backup", "Fallout4")).FullName;
            File.WriteAllText(Path.Combine(elsewhere, "Fallout4Prefs.ini"), "");
            var setup = new SetupViewModel(game, settingsFolder: MakeDocumentsFolder()) { InstallFolder = ChosenFolder };
            Assert.Contains("--settings-dir", setup.GameIniError);
            Assert.Contains(Path.Combine(setup.SettingsFolder, "Fallout4Prefs.ini"), setup.GameIniError);

            await setup.BrowseSettingsFolderAsync(new FakePicker(elsewhere));
            setup.Start();

            Assert.Equal("", setup.GameIniError);
            Assert.Equal(elsewhere, (await setup.Completed)!.SettingsFolder);
        }

        [Theory]
        [InlineData("", "Choose the Fallout 4 settings folder")]
        [InlineData("settings", "Enter a full path")]
        public void ABlankOrRelativeSettingsFolderIsRefused(string folder, string expectedStart)
        {
            string game = MakeGameFolder();
            File.WriteAllText(Path.Combine(game, "Fallout4_Default.ini"), "");
            var setup = new SetupViewModel(game, settingsFolder: MakeDocumentsFolder()) { InstallFolder = ChosenFolder };

            setup.SettingsFolder = folder;

            Assert.StartsWith(expectedStart, setup.GameIniError);
            Assert.False(setup.CanStart);
        }

        [Fact]
        public void TheShippedIniStaysTiedToTheGameFolderWhateverTheSettingsFolder()
        {
            string game = MakeGameFolder();
            string settings = MakeDocumentsFolder();
            File.WriteAllText(Path.Combine(settings, "Fallout4_Default.ini"), "");
            File.WriteAllText(Path.Combine(settings, "Fallout4Prefs.ini"), "");

            var setup = new SetupViewModel(game, settingsFolder: settings);

            Assert.StartsWith("The Fallout 4 installation looks incomplete.", setup.GameIniError);
            Assert.EndsWith($"Missing: {Path.Combine(game, "Fallout4_Default.ini")}.", setup.GameIniError);
        }

        [Fact]
        public void TheIniCheckWaitsForAValidGameFolder()
        {
            var setup = new SetupViewModel(Path.Combine(_root.Path, "missing"), settingsFolder: MakeDocumentsFolder());

            Assert.Equal("", setup.GameIniError);
            Assert.NotEqual("", setup.GameFolderError);
        }

        [Fact]
        public async Task BrowsingAfterTheStartChangesNothing()
        {
            string game = MakeGameFolder();
            var setup = new SetupViewModel(game);
            setup.InstallFolder = ChosenFolder;
            setup.Start();

            await setup.BrowseGameFolderAsync(new FakePicker(@"D:\Other"));

            Assert.Equal(game, setup.GameFolder);
        }
    }
}
