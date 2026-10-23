using Avalonia;
using Avalonia.Controls.ApplicationLifetimes;
using Avalonia.Threading;

namespace AssetConverter.InstallerUi
{
    public sealed class InstallerWindowException(string message, Exception inner) : Exception(message, inner);

    /// <summary>Runs the Avalonia application on its own STA thread so the install can use the process's main thread.</summary>
    internal sealed class InstallerWindowHost : IInstallerWindow
    {
        private static readonly TimeSpan StartupTimeout = TimeSpan.FromSeconds(30);

        private readonly TaskCompletionSource _opened = new(TaskCreationOptions.RunContinuationsAsynchronously);
        private readonly TaskCompletionSource _closed = new(TaskCreationOptions.RunContinuationsAsynchronously);
        private readonly InstallerViewModel _viewModel;

        private InstallerWindowHost(InstallerViewModel viewModel)
        {
            _viewModel = viewModel;
        }

        public Task Closed => _closed.Task;

        public Exception? Fault { get; private set; }

        /// <summary>Shows the window and returns once it is open; throws <see cref="InstallerWindowException"/> if it cannot be created.</summary>
        public static InstallerWindowHost Start(InstallerViewModel viewModel)
        {
            var host = new InstallerWindowHost(viewModel);
            var thread = new Thread(host.RunUi) { Name = "Installer UI", IsBackground = true };
            if (OperatingSystem.IsWindows()) thread.SetApartmentState(ApartmentState.STA);
            thread.Start();
            try
            {
                host._opened.Task.WaitAsync(StartupTimeout).GetAwaiter().GetResult();
            }
            catch (Exception ex)
            {
                host.Shutdown();
                string reason = ex is TimeoutException ? $"it did not open within {StartupTimeout.TotalSeconds:0} s" : ex.Message;
                throw new InstallerWindowException(
                    $"The installer window could not be opened ({reason}). Run with --headless (or OPENFO4_HEADLESS=1) to install without a window.", ex);
            }
            return host;
        }

        /// <summary>Like <see cref="Start"/>, but ends the process with a message saying why when the window cannot be opened.</summary>
        public static InstallerWindowHost StartOrExit(InstallerViewModel viewModel)
        {
            try
            {
                return Start(viewModel);
            }
            catch (InstallerWindowException ex)
            {
                Environment.Exit(InstallSession.ReportStartupFailure(new InstallStartupException(ex.Message, ex)));
                throw;
            }
        }

        public void RunOnUiThread(Action action)
        {
            if (Dispatcher.UIThread.CheckAccess())
            {
                action();
                return;
            }
            var ran = Dispatcher.UIThread.InvokeAsync(action).GetTask();
            if (Task.WhenAny(ran, _closed.Task).GetAwaiter().GetResult() != ran)
                throw new InstallerWindowException("The installer window was closed before the installation could start.", Fault ?? new InvalidOperationException("The window is gone."));
            ran.GetAwaiter().GetResult();
        }

        public void Shutdown() =>
            Dispatcher.UIThread.Post(() => (Application.Current?.ApplicationLifetime as IClassicDesktopStyleApplicationLifetime)?.Shutdown());

        private void RunUi()
        {
            try
            {
                AppBuilder.Configure(() => new InstallerApp(_viewModel, () => _opened.TrySetResult()))
                    .UsePlatformDetect()
                    .StartWithClassicDesktopLifetime([]);
                _opened.TrySetException(new InvalidOperationException("The installer window closed before it finished opening."));
            }
            catch (Exception ex)
            {
                Fault = ex;
                Console.Error.WriteLine($"ERROR: the installer window stopped unexpectedly: {ex}");
                _opened.TrySetException(ex);
            }
            finally
            {
                _closed.TrySetResult();
            }
        }
    }
}
