using System.ComponentModel;
using System.Globalization;
using AssetConverter.InstallerUi;
using AssetConverter.Logging;
using AssetConverter.Progress;
using Avalonia.Controls;
using Avalonia.Headless;
using Avalonia.Interactivity;
using Avalonia.Threading;
using InstallerTests.Support;
using Xunit;

namespace InstallerTests.Ui
{
    internal sealed class RecordingGameLauncher(Exception? failure = null) : IGameLauncher
    {
        public List<(string Executable, string WorkingDirectory)> Launches { get; } = [];

        public void Launch(string executable, string workingDirectory)
        {
            if (failure != null) throw failure;
            Launches.Add((executable, workingDirectory));
        }
    }

    public sealed class PlayViewModelTests : IDisposable
    {
        private readonly TempDirectory _root = new();
        private readonly ManualTimeProvider _clock = new();
        private readonly ProgressModel _model;
        private readonly RecordingGameLauncher _launcher = new();

        public PlayViewModelTests() => _model = new ProgressModel([new StepDefinition("Extracting", "files")], _clock);

        public void Dispose() => _root.Dispose();

        private string InstallRoot => Path.Combine(_root.Path, "OpenFO4");

        private string Exe => Path.Combine(InstallRoot, "openfo4.exe");

        private InstallerViewModel ViewModel(RecordingGameLauncher? launcher = null) =>
            new(_model.GetSnapshot, new LogBuffer(100), () => { }, Path.Combine(InstallRoot, "install.log"), CultureInfo.InvariantCulture, launcher ?? _launcher);

        private void InstallGame()
        {
            Directory.CreateDirectory(InstallRoot);
            File.WriteAllText(Exe, "exe");
        }

        [Fact]
        public void PlayIsHiddenWhileTheInstallRuns()
        {
            InstallGame();
            var viewModel = ViewModel();

            viewModel.Refresh();

            Assert.False(viewModel.CanPlay);
            Assert.False(viewModel.Play());
            Assert.Empty(_launcher.Launches);
        }

        [Fact]
        public void PlayShowsAfterASuccessfulInstallThatLeftTheGameExecutable()
        {
            InstallGame();
            var viewModel = ViewModel();
            var changed = new List<string?>();
            viewModel.PropertyChanged += (_, e) => changed.Add(e.PropertyName);

            _model.Finish(RunState.Succeeded);
            viewModel.Refresh();

            Assert.True(viewModel.CanPlay);
            Assert.Contains(nameof(InstallerViewModel.CanPlay), changed);
        }

        [Fact]
        public void PlayStaysHiddenWhenThereIsNoGameExecutableInTheInstallFolder()
        {
            Directory.CreateDirectory(InstallRoot);
            var viewModel = ViewModel();

            _model.Finish(RunState.Succeeded);
            viewModel.Refresh();

            Assert.False(viewModel.CanPlay);
        }

        [Theory]
        [InlineData(RunState.Failed)]
        [InlineData(RunState.Cancelled)]
        public void PlayStaysHiddenAfterAFailedOrCancelledInstallEvenWithAnEarlierExecutable(RunState state)
        {
            InstallGame();
            var viewModel = ViewModel();

            _model.Finish(state);
            viewModel.Refresh();

            Assert.False(viewModel.CanPlay);
        }

        [Fact]
        public void PlayStaysHiddenForAViewModelWithoutAnInstallLog()
        {
            var viewModel = new InstallerViewModel(_model.GetSnapshot, new LogBuffer(100), () => { }, culture: CultureInfo.InvariantCulture, gameLauncher: _launcher);

            _model.Finish(RunState.Succeeded);
            viewModel.Refresh();

            Assert.False(viewModel.CanPlay);
            Assert.Null(viewModel.GameExecutablePath);
        }

        [Fact]
        public void PlayLaunchesTheInstalledExecutableWithTheInstallFolderAsWorkingDirectory()
        {
            InstallGame();
            var viewModel = ViewModel();
            _model.Finish(RunState.Succeeded);
            viewModel.Refresh();

            bool launched = viewModel.Play();

            Assert.True(launched);
            Assert.Equal([(Exe, InstallRoot)], _launcher.Launches);
            Assert.Equal("", viewModel.PlayError);
        }

        [Theory]
        [InlineData(typeof(Win32Exception))]
        [InlineData(typeof(InvalidOperationException))]
        [InlineData(typeof(UnauthorizedAccessException))]
        [InlineData(typeof(IOException))]
        public void ALaunchThatFailsIsShownAsAnErrorAndDoesNotCountAsLaunched(Type failureType)
        {
            InstallGame();
            var viewModel = ViewModel(new RecordingGameLauncher((Exception)Activator.CreateInstance(failureType, "no can do")!));
            _model.Finish(RunState.Succeeded);
            viewModel.Refresh();

            bool launched = viewModel.Play();

            Assert.False(launched);
            Assert.Equal("The game could not be started: no can do", viewModel.PlayError);
            Assert.True(viewModel.CanPlay);
        }

        [Fact]
        public void ARetryAfterAFailureClearsTheError()
        {
            InstallGame();
            int attempts = 0;
            var viewModel = new InstallerViewModel(_model.GetSnapshot, new LogBuffer(100), () => { }, Path.Combine(InstallRoot, "install.log"),
                CultureInfo.InvariantCulture, new FlakyLauncher(() => attempts++ == 0));
            _model.Finish(RunState.Succeeded);
            viewModel.Refresh();

            Assert.False(viewModel.Play());
            Assert.NotEqual("", viewModel.PlayError);
            Assert.True(viewModel.Play());
            Assert.Equal("", viewModel.PlayError);
        }

        private sealed class FlakyLauncher(Func<bool> fails) : IGameLauncher
        {
            public void Launch(string executable, string workingDirectory)
            {
                if (fails()) throw new Win32Exception("not yet");
            }
        }
    }

    [Collection("Avalonia")]
    public sealed class PlayWindowTests : IDisposable
    {
        private readonly HeadlessUnitTestSession _session = HeadlessSessions.Shared;
        private readonly TempDirectory _root = new();
        private readonly ManualTimeProvider _clock = new();
        private readonly ProgressModel _model;

        public PlayWindowTests() => _model = new ProgressModel([new StepDefinition("Extracting", "files")], _clock);

        public void Dispose() => _root.Dispose();

        private string InstallRoot => Path.Combine(_root.Path, "OpenFO4");

        private Task<T> OnUiThread<T>(Func<T> action) => _session.Dispatch(action, CancellationToken.None);

        private InstallerWindow ShowFinished(IGameLauncher launcher, bool withExecutable, RunState state = RunState.Succeeded)
        {
            Directory.CreateDirectory(InstallRoot);
            if (withExecutable) File.WriteAllText(Path.Combine(InstallRoot, "openfo4.exe"), "exe");
            var viewModel = new InstallerViewModel(_model.GetSnapshot, new LogBuffer(100), () => { }, Path.Combine(InstallRoot, "install.log"), CultureInfo.InvariantCulture, launcher);
            var window = new InstallerWindow { DataContext = viewModel };
            window.Show();
            _model.Finish(state);
            viewModel.Refresh();
            Dispatcher.UIThread.RunJobs();
            window.UpdateLayout();
            return window;
        }

        private static void Click(Button button) => button.RaiseEvent(new RoutedEventArgs(Button.ClickEvent));

        [Fact]
        public async Task PlayAppearsBesideCloseOnlyAfterASuccessfulInstallWithAGameAndTheOtherButtonsStay()
        {
            var launcher = new RecordingGameLauncher();
            var (playVisible, closeVisible, folderVisible, logVisible) = await OnUiThread(() =>
            {
                var window = ShowFinished(launcher, withExecutable: true);
                return (window.FindControl<Button>("PlayButton")!.IsVisible, window.FindControl<Button>("ActionButton")!.IsVisible,
                    window.FindControl<Button>("OpenInstallFolderButton")!.IsVisible, window.FindControl<Button>("OpenLogButton")!.IsVisible);
            });

            Assert.True(playVisible);
            Assert.True(closeVisible);
            Assert.True(folderVisible);
            Assert.True(logVisible);
        }

        [Fact]
        public async Task PlayIsAbsentWithoutAGameExecutableAndAfterAFailedInstall()
        {
            var launcher = new RecordingGameLauncher();
            bool noExe = await OnUiThread(() => ShowFinished(launcher, withExecutable: false).FindControl<Button>("PlayButton")!.IsVisible);
            var afterFailure = await OnUiThread(() =>
            {
                var model = new ProgressModel([new StepDefinition("Extracting", "files")], _clock);
                File.WriteAllText(Path.Combine(InstallRoot, "openfo4.exe"), "exe");
                var viewModel = new InstallerViewModel(model.GetSnapshot, new LogBuffer(100), () => { }, Path.Combine(InstallRoot, "install.log"), CultureInfo.InvariantCulture, launcher);
                var window = new InstallerWindow { DataContext = viewModel };
                window.Show();
                model.Finish(RunState.Failed);
                viewModel.Refresh();
                Dispatcher.UIThread.RunJobs();
                return window.FindControl<Button>("PlayButton")!.IsVisible;
            });

            Assert.False(noExe);
            Assert.False(afterFailure);
        }

        [Fact]
        public async Task ClickingPlayLaunchesTheGameAndClosesTheWindow()
        {
            var launcher = new RecordingGameLauncher();
            var (stillOpen, launches) = await OnUiThread(() =>
            {
                var window = ShowFinished(launcher, withExecutable: true);
                bool closed = false;
                window.Closed += (_, _) => closed = true;

                Click(window.FindControl<Button>("PlayButton")!);
                Dispatcher.UIThread.RunJobs();

                return (!closed, launcher.Launches.ToList());
            });

            Assert.False(stillOpen);
            Assert.Equal([(Path.Combine(InstallRoot, "openfo4.exe"), InstallRoot)], launches);
        }

        [Fact]
        public async Task AFailedLaunchShowsTheErrorAndKeepsTheWindowOpen()
        {
            var launcher = new RecordingGameLauncher(new Win32Exception("access denied"));
            var (closed, errorText, errorVisible, playStillThere) = await OnUiThread(() =>
            {
                var window = ShowFinished(launcher, withExecutable: true);
                bool wasClosed = false;
                window.Closed += (_, _) => wasClosed = true;

                Click(window.FindControl<Button>("PlayButton")!);
                Dispatcher.UIThread.RunJobs();
                window.UpdateLayout();

                var error = window.FindControl<TextBlock>("PlayErrorText")!;
                return (wasClosed, error.Text, error.IsEffectivelyVisible, window.FindControl<Button>("PlayButton")!.IsVisible);
            });

            Assert.False(closed);
            Assert.Equal("The game could not be started: access denied", errorText);
            Assert.True(errorVisible);
            Assert.True(playStillThere);
        }
    }
}
