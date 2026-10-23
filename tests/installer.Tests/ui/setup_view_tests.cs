using System.Globalization;
using AssetConverter.InstallerUi;
using AssetConverter.Logging;
using AssetConverter.Progress;
using Avalonia;
using Avalonia.Controls;
using Avalonia.Headless;
using Avalonia.Interactivity;
using Avalonia.Threading;
using Avalonia.VisualTree;
using Xunit;

namespace InstallerTests.Ui
{
    /// <summary>The window on its setup view, running for real on Avalonia's headless platform.</summary>
    [Collection("Avalonia")]
    public sealed class SetupViewTests : IDisposable
    {
        private const string KeepLabel = "Keep temporary files (uses more storage, speeds up future installs)";

        private readonly HeadlessUnitTestSession _session;
        private readonly TempDirectory _root = new();

        public SetupViewTests() => _session = HeadlessSessions.Shared;

        public void Dispose() => _root.Dispose();

        private Task<T> OnUiThread<T>(Func<T> action) => _session.Dispatch(action, CancellationToken.None);

        private string MakeGameFolder()
        {
            string game = Path.Combine(_root.Path, "Fallout 4");
            Directory.CreateDirectory(Path.Combine(game, "Data"));
            File.WriteAllBytes(Path.Combine(game, "Fallout4.exe"), []);
            File.WriteAllBytes(Path.Combine(game, "Data", "Fallout4.esm"), []);
            return game;
        }

        private static InstallerWindow Show(InstallerViewModel viewModel)
        {
            var window = new InstallerWindow { DataContext = viewModel };
            window.Show();
            Pump(window);
            return window;
        }

        private static void Pump(Window window)
        {
            window.UpdateLayout();
            Dispatcher.UIThread.RunJobs();
            window.UpdateLayout();
        }

        private static SetupView ViewOf(Window window) => window.GetVisualDescendants().OfType<SetupView>().Single();

        private static T Control<T>(Window window, string name) where T : Control => ViewOf(window).FindControl<T>(name)!;

        private static void Click(Window window, Button button)
        {
            button.RaiseEvent(new RoutedEventArgs(Button.ClickEvent));
            Pump(window);
        }

        private static string[] TextOf(Visual root) => [.. root.GetVisualDescendants().OfType<TextBlock>().Select(t => t.Text ?? "")];

        [Fact]
        public async Task TheWindowOpensOnTheSetupViewAndStartIsDisabledUntilBothFoldersAreValid()
        {
            var (setupVisible, progressVisible, startAtFirst, startWithGame, startWithBoth, errorsAtFirst) = await OnUiThread(() =>
            {
                var setup = new SetupViewModel();
                var window = Show(new InstallerViewModel(setup, CultureInfo.InvariantCulture));
                var start = Control<Button>(window, "StartButton");
                var result = (ViewOf(window).IsEffectivelyVisible, window.FindControl<Button>("ActionButton")!.IsEffectivelyVisible,
                    start.IsEffectivelyEnabled, false, false, TextOf(ViewOf(window)));

                Control<TextBox>(window, "GameFolderBox").Text = MakeGameFolder();
                Pump(window);
                result.Item4 = start.IsEffectivelyEnabled;
                Control<TextBox>(window, "InstallFolderBox").Text = _root.Path;
                Pump(window);
                result.Item5 = start.IsEffectivelyEnabled;
                window.Close();
                return result;
            });

            Assert.True(setupVisible);
            Assert.False(progressVisible);
            Assert.False(startAtFirst);
            Assert.False(startWithGame);
            Assert.True(startWithBoth);
            Assert.Contains("Choose the Fallout 4 folder.", errorsAtFirst);
            Assert.Contains("Choose where to install.", errorsAtFirst);
        }

        [Fact]
        public async Task AnInvalidFolderShowsItsMessageBesideTheFieldAndAValidOneShowsNone()
        {
            var (gameError, installError, afterFix) = await OnUiThread(() =>
            {
                var setup = new SetupViewModel(Path.Combine(_root.Path, "nowhere"));
                var window = Show(new InstallerViewModel(setup, CultureInfo.InvariantCulture));
                Control<TextBox>(window, "InstallFolderBox").Text = "relative";
                Pump(window);
                var gameText = Control<TextBlock>(window, "GameFolderErrorText").Text;
                var installText = Control<TextBlock>(window, "InstallFolderErrorText").Text;
                Control<TextBox>(window, "GameFolderBox").Text = MakeGameFolder();
                Pump(window);
                var fixedText = Control<TextBlock>(window, "GameFolderErrorText").Text;
                window.Close();
                return (gameText, installText, fixedText);
            });

            Assert.Equal("This folder does not exist.", gameError);
            Assert.Equal(@"Enter a full path, such as D:\Games.", installError);
            Assert.Equal("", afterFix);
        }

        [Fact]
        public async Task TheKeepCheckboxCarriesTheExactLabelIsUncheckedByDefaultAndItsStateReachesTheChoice()
        {
            var (label, checkedAtFirst, choice) = await OnUiThread(() =>
            {
                var setup = new SetupViewModel(MakeGameFolder());
                var window = Show(new InstallerViewModel(setup, CultureInfo.InvariantCulture));
                var box = Control<CheckBox>(window, "KeepStagingBox");
                var result = ((string)box.Content!, box.IsChecked == true);
                Control<TextBox>(window, "InstallFolderBox").Text = _root.Path;
                box.IsChecked = true;
                Click(window, Control<Button>(window, "StartButton"));
                return (result.Item1, result.Item2, setup.Completed.Result);
            });

            Assert.Equal(KeepLabel, label);
            Assert.False(checkedAtFirst);
            Assert.Equal(new SetupChoice(Path.Combine(_root.Path, "Fallout 4"), Path.Combine(_root.Path, "OpenFO4"), KeepStaging: true), choice);
        }

        [Fact]
        public async Task TheCheckboxStartsCheckedWhenTheSetupIsPrefilledToKeep()
        {
            bool isChecked = await OnUiThread(() =>
            {
                var window = Show(new InstallerViewModel(new SetupViewModel(keepStaging: true), CultureInfo.InvariantCulture));
                bool result = Control<CheckBox>(window, "KeepStagingBox").IsChecked == true;
                window.Close();
                return result;
            });

            Assert.True(isChecked);
        }

        [Fact]
        public async Task PressingStartCompletesTheSetupAndLocksTheViewUntilTheInstallTakesOver()
        {
            var (completed, startEnabledAfter, setupStillShown) = await OnUiThread(() =>
            {
                var setup = new SetupViewModel(MakeGameFolder());
                var window = Show(new InstallerViewModel(setup, CultureInfo.InvariantCulture));
                Control<TextBox>(window, "InstallFolderBox").Text = _root.Path;
                Pump(window);
                Click(window, Control<Button>(window, "StartButton"));
                var result = (setup.Completed.IsCompletedSuccessfully, Control<Button>(window, "StartButton").IsEffectivelyEnabled, ViewOf(window).IsEffectivelyVisible);
                return result;
            });

            Assert.True(completed);
            Assert.False(startEnabledAfter);
            Assert.True(setupStillShown);
        }

        [Fact]
        public async Task OnceTheInstallBeginsTheWindowSwitchesToTheProgressView()
        {
            var (setupVisible, progressVisible, texts, actionLabel) = await OnUiThread(() =>
            {
                var viewModel = new InstallerViewModel(new SetupViewModel(MakeGameFolder()), CultureInfo.InvariantCulture);
                var window = Show(viewModel);
                var model = new ProgressModel([new StepDefinition("Extracting", "files")]);
                string logFile = Path.Combine(_root.Path, "OpenFO4", "install.log");

                viewModel.BeginInstall(model.GetSnapshot, new LogBuffer(InstallerViewModel.LogCapacity), () => { }, logFile);
                viewModel.Refresh();
                Pump(window);
                var action = window.FindControl<Button>("ActionButton")!;
                var result = (ViewOf(window).IsEffectivelyVisible, action.IsEffectivelyVisible, TextOf(window), (string)action.Content!);
                model.Finish(RunState.Succeeded);
                viewModel.Refresh();
                window.Close();
                return result;
            });

            Assert.False(setupVisible);
            Assert.True(progressVisible);
            Assert.Contains("Extracting", texts);
            Assert.Contains($"Log file: {Path.Combine(_root.Path, "OpenFO4", "install.log")}", texts);
            Assert.Equal("Cancel", actionLabel);
        }

        [Fact]
        public async Task TheCancelButtonOnTheSetupViewEndsTheSetupWithoutAChoiceAndClosesTheWindow()
        {
            var (choice, windowOpen) = await OnUiThread(() =>
            {
                var setup = new SetupViewModel(MakeGameFolder());
                var window = Show(new InstallerViewModel(setup, CultureInfo.InvariantCulture));

                Click(window, Control<Button>(window, "CancelSetupButton"));
                return (setup.Completed.Result, window.IsVisible);
            });

            Assert.Null(choice);
            Assert.False(windowOpen);
        }

        [Fact]
        public async Task ClosingTheWindowOnTheSetupViewEndsTheSetupWithoutAChoiceAndAsksNothing()
        {
            var (choice, windowOpen, dialogs) = await OnUiThread(() =>
            {
                var setup = new SetupViewModel(MakeGameFolder());
                var window = Show(new InstallerViewModel(setup, CultureInfo.InvariantCulture));

                window.Close();
                Pump(window);
                return (setup.Completed.Result, window.IsVisible, window.OwnedWindows.Count);
            });

            Assert.Null(choice);
            Assert.False(windowOpen);
            Assert.Equal(0, dialogs);
        }

        [Fact]
        public async Task ClosingTheWindowAfterStartAsksFirstAndDoesNotRetractTheChoice()
        {
            var (choice, dialogsOpen, windowStillOpen) = await OnUiThread(() =>
            {
                var (setup, window) = ShowAndPressStart();

                window.Close();
                Pump(window);
                var result = (setup.Completed.Result, window.OwnedWindows.Count, window.IsVisible);
                window.OwnedWindows[0].Close();
                Pump(window);
                return result;
            });

            Assert.NotNull(choice);
            Assert.Equal(1, dialogsOpen);
            Assert.True(windowStillOpen);
        }

        [Fact]
        public async Task ConfirmingTheCancellationAfterStartCancelsTheInstallWhenItBegins()
        {
            var (cancelRequestedAtOnce, cancelCalls) = await OnUiThread(() =>
            {
                var (_, window) = ShowAndPressStart();
                var viewModel = (InstallerViewModel)window.DataContext!;
                window.Close();
                Pump(window);
                Click(window.OwnedWindows[0], window.OwnedWindows[0].GetVisualDescendants().OfType<Button>().Single(b => (string?)b.Content == "Cancel installation"));
                int calls = 0;
                bool requested = viewModel.CancelRequested;
                viewModel.BeginInstall(new ProgressModel([new StepDefinition("Extracting", "files")]).GetSnapshot, new LogBuffer(InstallerViewModel.LogCapacity), () => calls++, Path.Combine(_root.Path, "OpenFO4", "install.log"));
                viewModel.Refresh();
                Pump(window);
                window.Close();
                Pump(window);
                return (requested, calls);
            });

            Assert.True(cancelRequestedAtOnce);
            Assert.Equal(1, cancelCalls);
        }

        private (SetupViewModel Setup, InstallerWindow Window) ShowAndPressStart()
        {
            var setup = new SetupViewModel(MakeGameFolder());
            var window = Show(new InstallerViewModel(setup, CultureInfo.InvariantCulture));
            Control<TextBox>(window, "InstallFolderBox").Text = _root.Path;
            Pump(window);
            Click(window, Control<Button>(window, "StartButton"));
            return (setup, window);
        }

        [Fact]
        public async Task AnInstallFolderThatCannotBeCreatedIsShownOnTheSetupViewAndTheViewStaysEditable()
        {
            string blocker = Path.Combine(_root.Path, "blocker");
            File.WriteAllText(blocker, "a file");

            var (startError, completed, startEnabled) = await OnUiThread(() =>
            {
                var setup = new SetupViewModel(MakeGameFolder());
                var window = Show(new InstallerViewModel(setup, CultureInfo.InvariantCulture));
                Control<TextBox>(window, "InstallFolderBox").Text = Path.Combine(blocker, "install");
                Pump(window);
                Click(window, Control<Button>(window, "StartButton"));
                return (Control<TextBlock>(window, "StartErrorText").Text, setup.Completed.IsCompleted, Control<Button>(window, "StartButton").IsEffectivelyEnabled);
            });

            Assert.StartsWith("Cannot create the install folder", startError, StringComparison.Ordinal);
            Assert.False(completed);
            Assert.True(startEnabled);
        }
    }
}
