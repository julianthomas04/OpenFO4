using AssetConverter;
using AssetConverter.InstallerUi;
using AssetConverter.Logging;
using AssetConverter.Pipeline;
using AssetConverter.Progress;
using Avalonia.Controls;
using Avalonia.Headless;
using Avalonia.Threading;
using Xunit;

namespace InstallerTests.Ui
{
    /// <summary>
    /// The install starting in a window that shows the setup, as in a real run: the install thread is not the UI thread, and the
    /// window's bindings accept changes from the UI thread only.
    /// </summary>
    [Collection("Console redirection")]
    public sealed class InstallSessionWindowTests : IDisposable
    {
        private readonly HeadlessUnitTestSession _session;
        private readonly string _directory = Directory.CreateTempSubdirectory("installer-tests-session-window-").FullName;

        public InstallSessionWindowTests() => _session = HeadlessSessions.Shared;

        public void Dispose()
        {
            InstallProgress.Stop();
            Directory.Delete(_directory, recursive: true);
        }

        private string LogPath => Path.Combine(_directory, "install.log");

        private Task<T> OnUiThread<T>(Func<T> action) => _session.Dispatch(action, CancellationToken.None);

        private sealed class HeadlessWindow(HeadlessUnitTestSession session) : IInstallerWindow
        {
            public Task Closed => Task.CompletedTask;

            public Exception? Fault => null;

            public void RunOnUiThread(Action action) => session.Dispatch(() =>
            {
                action();
                return 0;
            }, CancellationToken.None).GetAwaiter().GetResult();

            public void Shutdown()
            {
            }
        }

        private async Task<(InstallerViewModel ViewModel, InstallerWindow Window)> ShowSetupWindow() =>
            await OnUiThread(() =>
            {
                var viewModel = new InstallerViewModel(new SetupViewModel());
                var window = new InstallerWindow { DataContext = viewModel };
                window.Show();
                window.UpdateLayout();
                Dispatcher.UIThread.RunJobs();
                return (viewModel, window);
            });

        [Fact]
        public async Task TheInstallStartingOnAnotherThreadSwitchesTheOpenWindowToTheProgressView()
        {
            var (viewModel, window) = await ShowSetupWindow();

            var changedOn = new List<int>();
            int uiThreadId = await OnUiThread(() =>
            {
                viewModel.PropertyChanged += (_, _) => changedOn.Add(Environment.CurrentManagedThreadId);
                return Environment.CurrentManagedThreadId;
            });

            using var session = await Task.Run(() => InstallSession.StartInOpenWindow(viewModel, new HeadlessWindow(_session), LogPath, new InstallOptions()));

            Assert.NotEmpty(changedOn);
            Assert.All(changedOn, id => Assert.Equal(uiThreadId, id));
            var (progressVisible, setupVisible) = await OnUiThread(() =>
            {
                Dispatcher.UIThread.RunJobs();
                window.UpdateLayout();
                var result = (window.FindControl<Button>("ActionButton")!.IsEffectivelyVisible, window.FindControl<Button>("StartButton")?.IsEffectivelyVisible ?? false);
                InstallProgress.Model!.Finish(RunState.Succeeded);
                viewModel.Refresh();
                window.Close();
                return result;
            });
            Assert.True(progressVisible);
            Assert.False(setupVisible);
        }
    }
}
