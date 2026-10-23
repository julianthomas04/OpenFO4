using System.Diagnostics.CodeAnalysis;
using AssetConverter.InstallerUi;
using AssetConverter.Logging;
using AssetConverter.Pipeline;
using AssetConverter.Progress;

namespace AssetConverter
{
    /// <summary>
    /// The lifetime of one full install: console capture into the log buffer and install.log, the progress model, the step reporter and,
    /// unless headless, the installer window. <see cref="Finish"/> closes the run and returns the process exit code.
    /// </summary>
    internal sealed class InstallSession : IDisposable
    {
        public const int ExitSucceeded = 0;
        public const int ExitFailed = 1;
        public const int ExitCancelled = 2;

        private readonly ConsoleCapture _capture;
        private readonly LogFileWriter _logFile;
        private readonly StepReporter _reporter;
        private readonly IInstallerWindow? _window;
        private readonly ConsoleCancelEventHandler _interruptHandler;
        private int _interrupts;
        private bool _disposed;

        private InstallSession(ConsoleCapture capture, LogFileWriter logFile, StepReporter reporter, IInstallerWindow? window)
        {
            _capture = capture;
            _logFile = logFile;
            _reporter = reporter;
            _window = window;
            _interruptHandler = (_, e) => e.Cancel = HandleInterrupt();
            Console.CancelKeyPress += _interruptHandler;
        }

        /// <summary>
        /// Starts the session, or ends the process with <see cref="ExitFailed"/> and a message saying why it could not start
        /// (the install log cannot be written, the window cannot be opened).
        /// </summary>
        public static InstallSession StartOrExit(bool headless, string logFilePath, InstallOptions options)
        {
            try
            {
                return Start(headless, logFilePath, options: options);
            }
            catch (InstallStartupException ex)
            {
                Environment.Exit(ReportStartupFailure(ex));
                throw;
            }
        }

        /// <summary>
        /// Starts the session in a window already open on its setup view, which switches to the progress view. When the session cannot
        /// start, the window shows the failed run with the reason (and install.log holds it, when it can be written) and
        /// <paramref name="session"/> is null; the caller waits for the user to close the window.
        /// </summary>
        public static bool TryStartInOpenWindow(InstallerViewModel viewModel, IInstallerWindow window, string logFilePath, InstallOptions options,
            [NotNullWhen(true)] out InstallSession? session)
        {
            try
            {
                session = StartInOpenWindow(viewModel, window, logFilePath, options);
                return true;
            }
            catch (InstallStartupException ex)
            {
                ReportStartupFailure(ex);
                ShowStartupFailure(viewModel, window, logFilePath, options, ex);
                session = null;
                return false;
            }
        }

        private static void ShowStartupFailure(InstallerViewModel viewModel, IInstallerWindow window, string logFilePath, InstallOptions options, InstallStartupException ex)
        {
            var buffer = new LogBuffer(InstallerViewModel.LogCapacity);
            var model = new ProgressModel(InstallSteps.Planned(options.KeepStaging, options.UnpacksPayload));
            model.Finish(RunState.Failed);
            string line = $"ERROR: {ex.Message}";
            buffer.WriteLine(line, isError: true);
            try
            {
                using var logFile = new LogFileWriter(logFilePath, Console.Error.WriteLine);
                logFile.WriteLine(line, isError: true);
            }
            catch (Exception logEx) when (logEx is IOException or UnauthorizedAccessException)
            {
                Console.Error.WriteLine($"ERROR: the failure could not be written to '{logFilePath}': {logEx.Message}");
            }
            try
            {
                window.RunOnUiThread(() => viewModel.BeginInstall(model.GetSnapshot, buffer, () => { }, logFilePath));
            }
            catch (Exception windowEx) when (windowEx is InstallerWindowException or InvalidOperationException)
            {
                Console.Error.WriteLine($"ERROR: the failure could not be shown in the installer window: {windowEx.Message}");
                window.Shutdown();
            }
        }

        /// <summary>Prints the failure to stderr and returns the exit code; when <paramref name="logFilePath"/> is given, the failure is appended to that log as well.</summary>
        internal static int ReportStartupFailure(InstallStartupException ex, string? logFilePath = null)
        {
            string message = $"ERROR: {ex.Message}";
            Console.Error.WriteLine(message);
            if (ex.InnerException != null) Console.Error.WriteLine($"  Cause: {ex.InnerException.GetType().Name}: {ex.InnerException.Message}");
            if (logFilePath != null) AppendToLog(logFilePath, message);
            return ExitFailed;
        }

        private static void AppendToLog(string logFilePath, string message)
        {
            try
            {
                File.AppendAllText(logFilePath, new LogLine(0, DateTimeOffset.UtcNow, IsError: true, message).Format() + Environment.NewLine);
            }
            catch (Exception ex) when (ex is IOException or UnauthorizedAccessException)
            {
                Console.Error.WriteLine($"ERROR: the failure could not be written to '{logFilePath}': {ex.Message}");
            }
        }

        /// <exception cref="InstallStartupException">The session could not be started; everything started so far has been shut down again.</exception>
        internal static InstallSession Start(bool headless, string logFilePath, Func<InstallerViewModel, IInstallerWindow>? openWindow = null, InstallOptions? options = null) =>
            Start(logFilePath, options ?? new InstallOptions(), (model, buffer) =>
                headless ? null : (openWindow ?? InstallerWindowHost.Start)(
                    new InstallerViewModel(model.GetSnapshot, buffer, RequestCancelInBackground, logFilePath)));

        /// <exception cref="InstallStartupException">The session could not be started; everything started so far has been shut down again.</exception>
        internal static InstallSession StartInOpenWindow(InstallerViewModel viewModel, IInstallerWindow window, string logFilePath, InstallOptions options) =>
            Start(logFilePath, options, (model, buffer) =>
            {
                window.RunOnUiThread(() => viewModel.BeginInstall(model.GetSnapshot, buffer, RequestCancelInBackground, logFilePath));
                return window;
            });

        private static InstallSession Start(string logFilePath, InstallOptions options, Func<ProgressModel, LogBuffer, IInstallerWindow?> attachWindow)
        {
            var buffer = new LogBuffer(InstallerViewModel.LogCapacity);
            ConsoleCapture? capture = null;
            var logFile = OpenLogFile(logFilePath, message => (capture?.OriginalError ?? Console.Error).WriteLine(message));
            StepReporter? reporter = null;
            IInstallerWindow? window = null;
            try
            {
                capture = ConsoleCapture.Start(new CompositeLineSink(buffer, logFile));
                var model = InstallProgress.Start(InstallSteps.Planned(options.KeepStaging, options.UnpacksPayload));
                window = attachWindow(model, buffer);
                reporter = new StepReporter(model, Console.Out);
                reporter.Start();
                return new InstallSession(capture, logFile, reporter, window);
            }
            catch (Exception ex)
            {
                window?.Shutdown();
                reporter?.DisposeAsync().AsTask().GetAwaiter().GetResult();
                InstallProgress.Stop();
                capture?.Dispose();
                logFile.Dispose();
                throw ex as InstallStartupException ?? DescribeStartupFailure(ex);
            }
        }

        /// <summary>Cancels the install without running the cancellation callbacks on the caller's thread, which is the UI thread when the user clicks Cancel.</summary>
        internal static void RequestCancelInBackground() =>
            TaskFaultReporter.Observe(Task.Run(InstallProgress.RequestCancel), "cancelling the installation");

        /// <summary>
        /// Closes the run with the exception that ended it (null on success), reports the outcome, waits for the window to be closed
        /// and returns the exit code.
        /// </summary>
        public int Finish(Exception? ended)
        {
            RunState outcome = InstallProgress.Conclude(ended);
            _reporter.DisposeAsync().AsTask().GetAwaiter().GetResult();
            Console.WriteLine();
            Console.WriteLine(outcome switch
            {
                RunState.Succeeded => "Installation finished.",
                RunState.Cancelled => "Installation cancelled.",
                _ => "Installation FAILED.",
            });
            Console.WriteLine($"Log file: {_logFile.Path}");
            int exitCode = outcome switch
            {
                RunState.Succeeded => ExitSucceeded,
                RunState.Cancelled => ExitCancelled,
                _ => ExitFailed,
            };
            return AwaitWindowClosed() ? exitCode : ExitFailed;
        }

        public void Dispose()
        {
            if (_disposed) return;
            _disposed = true;
            Console.CancelKeyPress -= _interruptHandler;
            if (_window is { Closed.IsCompleted: false }) _window.Shutdown();
            _reporter.DisposeAsync().AsTask().GetAwaiter().GetResult();
            InstallProgress.Stop();
            _capture.Dispose();
            _logFile.Dispose();
        }

        /// <summary>
        /// The first Ctrl+C cancels the install gracefully (stopping worker processes and leaving resumable state); a second one
        /// leaves the process's default handling to end it at once. Returns whether the key press was handled.
        /// </summary>
        internal bool HandleInterrupt()
        {
            if (Interlocked.Increment(ref _interrupts) > 1) return false;
            Console.Error.WriteLine("Interrupted: cancelling the installation. Press Ctrl+C again to quit immediately.");
            InstallProgress.RequestCancel();
            return true;
        }

        private bool AwaitWindowClosed()
        {
            if (_window == null) return true;
            _window.Closed.GetAwaiter().GetResult();
            if (_window.Fault == null) return true;
            Console.Error.WriteLine("ERROR: the installer window had stopped unexpectedly (see above).");
            return false;
        }

        private static LogFileWriter OpenLogFile(string path, Action<string> onFault)
        {
            try
            {
                return new LogFileWriter(path, onFault);
            }
            catch (Exception ex) when (ex is IOException or UnauthorizedAccessException)
            {
                throw new InstallStartupException(
                    $"Cannot write the install log '{path}': {ex.Message} Another installer may be running, or the folder is read-only.", ex);
            }
        }

        private static InstallStartupException DescribeStartupFailure(Exception ex) =>
            ex is InstallerWindowException ? new(ex.Message, ex) : new($"The installer could not start: {ex.Message}", ex);
    }
}
