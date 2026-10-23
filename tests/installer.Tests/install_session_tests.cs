using AssetConverter;
using AssetConverter.InstallerUi;
using AssetConverter.Pipeline;
using AssetConverter.Progress;
using Xunit;

namespace InstallerTests
{
    /// <summary>The headless run of a full install: console capture into install.log, the step lines and the process exit code.</summary>
    [Collection("Console redirection")]
    public sealed class InstallSessionTests : IDisposable
    {
        private readonly string _directory = Directory.CreateTempSubdirectory("installer-tests-session-").FullName;
        private readonly TextWriter _originalOut = Console.Out;
        private readonly TextWriter _originalError = Console.Error;

        public void Dispose()
        {
            Console.SetOut(_originalOut);
            Console.SetError(_originalError);
            InstallProgress.Stop();
            Directory.Delete(_directory, recursive: true);
        }

        private string LogPath => Path.Combine(_directory, "install.log");

        private string FinishAndReadLog(InstallSession session, Exception? ended, out int exitCode)
        {
            exitCode = session.Finish(ended);
            session.Dispose();
            return File.ReadAllText(LogPath);
        }

        [Fact]
        public void ASuccessfulRunWritesTheConsoleOutputAndStepLinesToTheLogAndExitsZero()
        {
            var session = InstallSession.Start(headless: true, LogPath);
            Console.WriteLine("Unpacking archive");
            Console.Error.WriteLine("WARNING: something odd");
            InstallProgress.EndPhase(InstallProgress.BeginPhase(InstallSteps.ConvertMeshes, 5));

            string log = FinishAndReadLog(session, ended: null, out int exitCode);

            Assert.Equal(InstallSession.ExitSucceeded, exitCode);
            Assert.Contains("Unpacking archive", log, StringComparison.Ordinal);
            Assert.Contains("ERR WARNING: something odd", log, StringComparison.Ordinal);
            Assert.Contains($"Started: {InstallSteps.ConvertMeshes}", log, StringComparison.Ordinal);
            Assert.Contains($"Finished: {InstallSteps.ConvertMeshes}", log, StringComparison.Ordinal);
            Assert.Contains("Installation finished.", log, StringComparison.Ordinal);
            Assert.Same(_originalOut, Console.Out);
            Assert.Same(_originalError, Console.Error);
        }

        [Fact]
        public void AFailureIsLoggedWithItsExceptionAndExitsOne()
        {
            var session = InstallSession.Start(headless: true, LogPath);

            string log = FinishAndReadLog(session, new InvalidOperationException("converter exploded"), out int exitCode);

            Assert.Equal(InstallSession.ExitFailed, exitCode);
            Assert.Contains("converter exploded", log, StringComparison.Ordinal);
            Assert.Contains("Installation FAILED.", log, StringComparison.Ordinal);
        }

        [Fact]
        public void ACancelledRunExitsTwo()
        {
            var session = InstallSession.Start(headless: true, LogPath);
            InstallProgress.RequestCancel();

            string log = FinishAndReadLog(session, new OperationCanceledException(InstallProgress.CancelToken), out int exitCode);

            Assert.Equal(InstallSession.ExitCancelled, exitCode);
            Assert.Contains("Installation cancelled.", log, StringComparison.Ordinal);
        }

        [Fact]
        public void TheLogFileOfAPreviousRunIsReplaced()
        {
            File.WriteAllText(LogPath, "stale content from the previous run");

            var session = InstallSession.Start(headless: true, LogPath);
            string log = FinishAndReadLog(session, ended: null, out _);

            Assert.DoesNotContain("stale content", log, StringComparison.Ordinal);
        }

        private sealed class FakeWindow(bool alreadyClosed = true) : IInstallerWindow
        {
            private readonly TaskCompletionSource _closed = new(TaskCreationOptions.RunContinuationsAsynchronously);

            public int ShutdownCalls { get; private set; }

            public Task Closed => alreadyClosed ? Task.CompletedTask : _closed.Task;

            public Exception? Fault { get; init; }

            /// <summary>The thread the fake window's "UI" runs on, which is never the caller's.</summary>
            public int UiThreadId { get; private set; }

            public bool FailTheFirstRunOnUiThread { get; init; }

            private bool _failed;

            public void RunOnUiThread(Action action)
            {
                if (FailTheFirstRunOnUiThread && !_failed)
                {
                    _failed = true;
                    throw new InstallerWindowException("The installer window was closed before the installation could start.", new InvalidOperationException("the window is gone"));
                }
                Exception? thrown = null;
                var ui = new Thread(() =>
                {
                    UiThreadId = Environment.CurrentManagedThreadId;
                    try
                    {
                        action();
                    }
                    catch (Exception ex)
                    {
                        thrown = ex;
                    }
                });
                ui.Start();
                ui.Join();
                if (thrown != null) throw thrown;
            }

            public void Shutdown()
            {
                ShutdownCalls++;
                _closed.TrySetResult();
            }

            public void Close() => _closed.TrySetResult();
        }

        private InstallSession StartWindowed(FakeWindow window) => InstallSession.Start(headless: false, LogPath, _ => window);

        [Theory]
        [InlineData(null, InstallSession.ExitSucceeded)]
        [InlineData(typeof(InvalidOperationException), InstallSession.ExitFailed)]
        [InlineData(typeof(OperationCanceledException), InstallSession.ExitCancelled)]
        public void TheExitCodeOfAWindowedRunFollowsTheOutcome(Type? endedWith, int expected)
        {
            var session = StartWindowed(new FakeWindow());
            if (endedWith == typeof(OperationCanceledException)) InstallProgress.RequestCancel();
            var ended = endedWith == null ? null : (Exception)Activator.CreateInstance(endedWith)!;

            FinishAndReadLog(session, ended, out int exitCode);

            Assert.Equal(expected, exitCode);
        }

        [Theory]
        [InlineData(null)]
        [InlineData(typeof(OperationCanceledException))]
        public void ARunWhoseWindowCrashedExitsOneWhateverTheOutcomeWas(Type? endedWith)
        {
            var session = StartWindowed(new FakeWindow { Fault = new InvalidOperationException("ui exploded") });
            if (endedWith != null) InstallProgress.RequestCancel();
            var ended = endedWith == null ? null : (Exception)Activator.CreateInstance(endedWith)!;

            string log = FinishAndReadLog(session, ended, out int exitCode);

            Assert.Equal(InstallSession.ExitFailed, exitCode);
            Assert.Contains("the installer window had stopped unexpectedly", log, StringComparison.Ordinal);
        }

        [Fact]
        public async Task AWindowedRunWaitsForTheUserToCloseTheWindowBeforeReturningTheExitCode()
        {
            var window = new FakeWindow(alreadyClosed: false);
            var session = StartWindowed(window);

            var finishing = Task.Run(() => session.Finish(null), TestContext.Current.CancellationToken);
            window.Close();

            Assert.Equal(InstallSession.ExitSucceeded, await finishing.WaitAsync(TimeSpan.FromSeconds(30), TestContext.Current.CancellationToken));
            session.Dispose();
        }

        [Fact]
        public void DisposingShutsDownAWindowThatIsStillOpenButNotOneThatIsGone()
        {
            var open = new FakeWindow(alreadyClosed: false);
            StartWindowed(open).Dispose();
            var gone = new FakeWindow(alreadyClosed: true);
            StartWindowed(gone).Dispose();

            Assert.Equal(1, open.ShutdownCalls);
            Assert.Equal(0, gone.ShutdownCalls);
        }

        [Fact]
        public void AWindowThatCannotBeOpenedFailsTheStartWithItsMessageAndLeavesNothingBehind()
        {
            var windowError = new InstallerWindowException("The installer window could not be opened (no display).", new InvalidOperationException("no display"));

            var ex = Assert.Throws<InstallStartupException>(() => InstallSession.Start(headless: false, LogPath, _ => throw windowError));

            Assert.Equal(windowError.Message, ex.Message);
            AssertNothingLeftBehind();
        }

        [Fact]
        public void AnyOtherStartupErrorIsWrappedWithAnExplanation()
        {
            var ex = Assert.Throws<InstallStartupException>(() => InstallSession.Start(headless: false, LogPath, _ => throw new InvalidOperationException("boom")));

            Assert.Equal("The installer could not start: boom", ex.Message);
            Assert.IsType<InvalidOperationException>(ex.InnerException);
            AssertNothingLeftBehind();
        }

        [Fact]
        public void TheRemovalOfTheTemporaryFilesIsAStepOfTheRunOnlyWhenTheyAreNotKept()
        {
            using (InstallSession.Start(headless: true, LogPath, options: new InstallOptions(KeepStaging: true)))
            {
                Assert.Equal(InstallSteps.VisibleNames(InstallSteps.Planned(keepStaging: true)), InstallProgress.Model!.GetSnapshot().Steps.Select(s => s.Name));
                Assert.DoesNotContain(InstallProgress.Model!.GetSnapshot().Steps, step => step.Name == InstallSteps.RemoveStaging);
            }

            using (InstallSession.Start(headless: true, LogPath, options: new InstallOptions(KeepStaging: false)))
                Assert.Equal(InstallSteps.VisibleNames(InstallSteps.All), InstallProgress.Model!.GetSnapshot().Steps.Select(s => s.Name));
        }

        [Fact]
        public void ARunStartedInTheWindowThatShowedTheSetupSwitchesItToTheProgressOfTheNewRun()
        {
            var viewModel = new InstallerViewModel(new SetupViewModel());
            var window = new FakeWindow();

            using var session = InstallSession.StartInOpenWindow(viewModel, window, LogPath, new InstallOptions());
            viewModel.Refresh();

            Assert.False(viewModel.IsSettingUp);
            Assert.Equal(LogPath, viewModel.LogFilePath);
            Assert.Equal(_directory, viewModel.InstallFolderPath);
            Assert.Equal(InstallSteps.VisibleNames(InstallSteps.All).Count, viewModel.Steps.Count);
            Assert.Equal(0, window.ShutdownCalls);
        }

        [Fact]
        public void ARunStartedInTheWindowThatShowedTheSetupChangesTheViewModelOnTheWindowsThreadOnly()
        {
            var viewModel = new InstallerViewModel(new SetupViewModel());
            var window = new FakeWindow();
            var changedOn = new List<int>();
            viewModel.PropertyChanged += (_, _) => changedOn.Add(Environment.CurrentManagedThreadId);

            using var session = InstallSession.StartInOpenWindow(viewModel, window, LogPath, new InstallOptions());

            Assert.NotEmpty(changedOn);
            Assert.All(changedOn, id => Assert.Equal(window.UiThreadId, id));
            Assert.NotEqual(Environment.CurrentManagedThreadId, window.UiThreadId);
        }

        [Fact]
        public void AStartupFailureAfterInstallIsShownInTheWindowAsAFailedRunAndWrittenToTheLog()
        {
            var viewModel = new InstallerViewModel(new SetupViewModel());
            var window = new FakeWindow { FailTheFirstRunOnUiThread = true };

            bool started = InstallSession.TryStartInOpenWindow(viewModel, window, LogPath, new InstallOptions(), out var session);

            Assert.False(started);
            Assert.Null(session);
            viewModel.Refresh();
            Assert.False(viewModel.IsSettingUp);
            Assert.True(viewModel.IsFailed);
            Assert.True(viewModel.IsFinished);
            Assert.Contains(viewModel.LogLines, line => line.Text.Contains("closed before the installation could start", StringComparison.Ordinal));
            Assert.Contains("closed before the installation could start", File.ReadAllText(LogPath), StringComparison.Ordinal);
        }

        [Fact]
        public void AStartupFailureThatCannotBeWrittenToTheLogIsStillShownInTheWindow()
        {
            var viewModel = new InstallerViewModel(new SetupViewModel());
            var window = new FakeWindow();
            Directory.CreateDirectory(LogPath);
            string unwritableLog = LogPath;

            bool started = InstallSession.TryStartInOpenWindow(viewModel, window, unwritableLog, new InstallOptions(), out _);

            Assert.False(started);
            viewModel.Refresh();
            Assert.True(viewModel.IsFailed);
            Assert.Contains(viewModel.LogLines, line => line.Text.Contains("Cannot write the install log", StringComparison.Ordinal));
        }

        [Fact]
        public void ALogThatCannotBeOpenedLeavesTheSetupWindowAloneForTheCallerToReport()
        {
            var viewModel = new InstallerViewModel(new SetupViewModel());
            var window = new FakeWindow();
            using var otherInstaller = new FileStream(LogPath, FileMode.Create, FileAccess.Write, FileShare.None);

            Assert.Throws<InstallStartupException>(() => InstallSession.StartInOpenWindow(viewModel, window, LogPath, new InstallOptions()));

            Assert.True(viewModel.IsSettingUp);
        }

        [Fact]
        public void AnInstallLogThatCannotBeOpenedIsAFatalStartupErrorNamingThePath()
        {
            using var otherInstaller = new FileStream(LogPath, FileMode.Create, FileAccess.Write, FileShare.None);

            var ex = Assert.Throws<InstallStartupException>(() => InstallSession.Start(headless: true, LogPath));

            Assert.Contains(LogPath, ex.Message, StringComparison.Ordinal);
            Assert.IsAssignableFrom<IOException>(ex.InnerException);
            Assert.Same(_originalOut, Console.Out);
            Assert.Same(_originalError, Console.Error);
            Assert.Null(InstallProgress.Model);
        }

        [Fact]
        public void AStartupFailureIsReportedAsOneClearMessageAndExitCodeOne()
        {
            var error = new StringWriter();
            Console.SetError(error);
            var ex = new InstallStartupException("Cannot write the install log 'x'.", new IOException("sharing violation"));

            int exitCode = InstallSession.ReportStartupFailure(ex);

            Assert.Equal(InstallSession.ExitFailed, exitCode);
            string[] lines = error.ToString().Split(Environment.NewLine, StringSplitOptions.RemoveEmptyEntries);
            Assert.Equal(["ERROR: Cannot write the install log 'x'.", "  Cause: IOException: sharing violation"], lines);
        }

        [Fact]
        public void TheFirstInterruptCancelsTheInstallAndTheSecondOneLeavesTheDefaultTermination()
        {
            var session = InstallSession.Start(headless: true, LogPath);
            try
            {
                bool first = session.HandleInterrupt();
                bool cancelledAfterFirst = InstallProgress.CancelToken.IsCancellationRequested;
                bool second = session.HandleInterrupt();

                Assert.True(first);
                Assert.True(cancelledAfterFirst);
                Assert.False(second);
            }
            finally
            {
                session.Dispose();
            }
        }

        [Fact]
        public void AnInterruptedRunExitsAsCancelledEvenWhenWorkersFailWhileTheirProcessesAreKilled()
        {
            var session = InstallSession.Start(headless: true, LogPath);
            session.HandleInterrupt();

            string log = FinishAndReadLog(session, new IOException("pipe closed by killed worker"), out int exitCode);

            Assert.Equal(InstallSession.ExitCancelled, exitCode);
            Assert.Contains("Interrupted: cancelling the installation", log, StringComparison.Ordinal);
            Assert.Contains("Installation cancelled.", log, StringComparison.Ordinal);
        }

        [Fact]
        public void ACancelRequestedFromTheUiThreadRunsItsCallbacksOnAnotherThread()
        {
            var session = InstallSession.Start(headless: true, LogPath);
            try
            {
                int callbackThread = -1;
                using var callbackRan = new ManualResetEventSlim();
                InstallProgress.CancelToken.Register(() =>
                {
                    callbackThread = Environment.CurrentManagedThreadId;
                    callbackRan.Set();
                });

                InstallSession.RequestCancelInBackground();

                Assert.True(callbackRan.Wait(TimeSpan.FromSeconds(30), TestContext.Current.CancellationToken));
                Assert.NotEqual(Environment.CurrentManagedThreadId, callbackThread);
            }
            finally
            {
                session.Dispose();
            }
        }

        private void AssertNothingLeftBehind()
        {
            Assert.Same(_originalOut, Console.Out);
            Assert.Same(_originalError, Console.Error);
            Assert.Null(InstallProgress.Model);
            File.Delete(LogPath);
        }

        [Fact]
        public void TheRunUsesTheDeclaredStepsAndAFreshCancellationToken()
        {
            var session = InstallSession.Start(headless: true, LogPath);
            try
            {
                var steps = InstallProgress.Model!.GetSnapshot().Steps;

                Assert.Equal(InstallSteps.VisibleNames(InstallSteps.All), steps.Select(s => s.Name));
                Assert.False(InstallProgress.CancelToken.IsCancellationRequested);
            }
            finally
            {
                session.Dispose();
            }
        }

        [Fact]
        public void APackagedInstallerShowsTheUnpackingAsItsFirstStep()
        {
            using var session = InstallSession.Start(headless: true, LogPath, options: new InstallOptions(UnpacksPayload: true));

            var steps = InstallProgress.Model!.GetSnapshot().Steps;

            Assert.Equal(InstallSteps.Initializing, steps[0].Name);
            Assert.Equal(InstallSteps.UnpackPayload, steps[0].Parts[0].Name);
            Assert.Equal(InstallSteps.VisibleNames(InstallSteps.All).Count, steps.Count);
        }

        [Fact]
        public void AStartupFailureIsAppendedToTheInstallLogWhenOneIsGiven()
        {
            Console.SetError(new StringWriter());
            File.WriteAllText(LogPath, "earlier run\n");

            InstallSession.ReportStartupFailure(new InstallStartupException("Close OpenFO4 before reinstalling."), LogPath);

            string log = File.ReadAllText(LogPath);
            Assert.StartsWith("earlier run", log, StringComparison.Ordinal);
            Assert.Contains("ERR ERROR: Close OpenFO4 before reinstalling.", log, StringComparison.Ordinal);
        }

        [Fact]
        public void AStartupFailureThatCannotBeLoggedIsStillReportedWithTheReasonForTheLogFailure()
        {
            var error = new StringWriter();
            Console.SetError(error);
            string unwritable = Path.Combine(_directory, "missing-folder", "install.log");

            int exitCode = InstallSession.ReportStartupFailure(new InstallStartupException("boom"), unwritable);

            Assert.Equal(InstallSession.ExitFailed, exitCode);
            Assert.Contains("ERROR: boom", error.ToString(), StringComparison.Ordinal);
            Assert.Contains($"could not be written to '{unwritable}'", error.ToString(), StringComparison.Ordinal);
        }
    }
}
