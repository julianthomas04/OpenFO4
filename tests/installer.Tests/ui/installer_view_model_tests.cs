using System.ComponentModel;
using System.Globalization;
using AssetConverter.InstallerUi;
using AssetConverter.Logging;
using AssetConverter.Progress;
using InstallerTests.Support;
using Xunit;

namespace InstallerTests.Ui
{
    public class InstallerViewModelTests
    {
        private readonly ManualTimeProvider _clock = new();
        private readonly ProgressModel _model;
        private readonly LogBuffer _log = new(InstallerViewModel.LogCapacity + 100);
        private readonly InstallerViewModel _viewModel;
        private int _cancelRequests;

        public InstallerViewModelTests()
        {
            _model = new ProgressModel(
            [
                new StepDefinition("Extracting", "files"),
                new StepDefinition("Converting meshes", "meshes", Weight: 3),
                new StepDefinition("Finalizing", "tasks"),
            ], _clock);
            _viewModel = new InstallerViewModel(_model.GetSnapshot, _log, () => _cancelRequests++, @"C:\install\install.log", CultureInfo.InvariantCulture);
        }

        [Fact]
        public void BeforeAnythingRunsEveryStepIsPendingAndEmpty()
        {
            _viewModel.Refresh();

            Assert.Equal("0 of 3 steps done · Starting", _viewModel.OverallLabel);
            Assert.Equal("0%", _viewModel.OverallPercentText);
            Assert.Equal(3, _viewModel.Steps.Count);
            Assert.All(_viewModel.Steps, row => Assert.Equal((StepStatus.Pending, 0.0), (row.Status, row.Fraction)));
            Assert.Equal("Cancel", _viewModel.ActionButtonText);
            Assert.True(_viewModel.ActionButtonEnabled);
            Assert.False(_viewModel.IsFinished);
            Assert.Equal(@"Log file: C:\install\install.log", _viewModel.LogFileText);
        }

        [Fact]
        public void EveryStepExposesItsOwnFractionSoSeveralBarsRunAtOnce()
        {
            int extracting = _model.BeginPhase("Extracting", 10, null);
            _model.Tick(extracting, 4);
            int meshes = _model.BeginPhase("Converting meshes", 63_000, null);
            _model.Tick(meshes, 12_345);

            _viewModel.Refresh();

            Assert.Equal(0.4, _viewModel.Steps[0].Fraction, precision: 10);
            Assert.Equal(12_345d / 63_000, _viewModel.Steps[1].Fraction, precision: 10);
            Assert.Equal(0.0, _viewModel.Steps[2].Fraction);
            Assert.Equal("0 of 3 steps done · 2 running", _viewModel.OverallLabel);
        }

        [Fact]
        public void AStepWithoutATotalHasAnEmptyFractionWhileRunning()
        {
            int extracting = _model.BeginPhase("Extracting", 0, null);
            _model.Tick(extracting, 500);

            _viewModel.Refresh();

            Assert.Equal((StepStatus.Running, 0.0), (_viewModel.Steps[0].Status, _viewModel.Steps[0].Fraction));
        }

        [Fact]
        public void ARunningStepNeverReadsAsFullAndADoneStepAlwaysDoes()
        {
            int extracting = _model.BeginPhase("Extracting", 10, null);
            _model.Tick(extracting, 10);
            _viewModel.Refresh();
            double whileRunning = _viewModel.Steps[0].Fraction;

            _model.EndPhase(extracting);
            _viewModel.Refresh();

            Assert.Equal(ProgressModel.RunningFractionCap, whileRunning);
            Assert.Equal(1.0, _viewModel.Steps[0].Fraction);
        }

        [Fact]
        public void FailedAndCancelledStepsKeepTheFractionTheyReached()
        {
            int extracting = _model.BeginPhase("Extracting", 10, null);
            _model.Tick(extracting, 3);
            int meshes = _model.BeginPhase("Converting meshes", 100, null);
            _model.Tick(meshes, 25);
            _model.Fail(["Extracting"]);
            _model.Finish(RunState.Cancelled);

            _viewModel.Refresh();

            Assert.Equal((StepStatus.Failed, 0.3), (_viewModel.Steps[0].Status, _viewModel.Steps[0].Fraction));
            Assert.Equal((StepStatus.Cancelled, 0.25), (_viewModel.Steps[1].Status, _viewModel.Steps[1].Fraction));
            Assert.Equal((StepStatus.Cancelled, 0.0), (_viewModel.Steps[2].Status, _viewModel.Steps[2].Fraction));
        }

        [Fact]
        public void TheOverallBarFollowsTheWeightedProgress()
        {
            _model.EndPhase(_model.BeginPhase("Extracting", 0, null));
            int meshes = _model.BeginPhase("Converting meshes", 100, null);
            _model.Tick(meshes, 50);

            _viewModel.Refresh();

            Assert.Equal((1 + 3 * 0.5) / 5, _viewModel.OverallFraction, precision: 10);
            Assert.Equal("50%", _viewModel.OverallPercentText);
            Assert.Equal("1 of 3 steps done · 1 running", _viewModel.OverallLabel);
        }

        [Fact]
        public void TheStepListShowsEveryStepWithItsStatusCountAndTimes()
        {
            _model.EndPhase(_model.BeginPhase("Extracting", 10, null));
            int meshes = _model.BeginPhase("Converting meshes", 100, null);
            _model.Tick(meshes, 5);
            _clock.Advance(TimeSpan.FromSeconds(75));

            _viewModel.Refresh();

            var done = _viewModel.Steps[0];
            var running = _viewModel.Steps[1];
            var pending = _viewModel.Steps[2];
            Assert.Equal(("Extracting", StepStatus.Done, "✓"), (done.Name, done.Status, done.StatusGlyph));
            Assert.True(done.IsDone);
            Assert.True(running.IsRunning);
            Assert.Equal(0.05, running.Fraction, precision: 10);
            Assert.Equal("5 / 100", running.CountText);
            Assert.Equal("1m 15s", running.ElapsedText);
            Assert.Equal(("", ""), (pending.CountText, pending.ElapsedText));
        }

        [Fact]
        public void StepsAddedWhileRunningAppearInTheList()
        {
            _viewModel.Refresh();

            _model.BeginPhase("Surprise", 0, null);
            _viewModel.Refresh();

            Assert.Equal(4, _viewModel.Steps.Count);
            Assert.Equal("Surprise", _viewModel.Steps[3].Name);
        }

        [Fact]
        public void SuccessEnablesCloseAndCompletesTheBars()
        {
            _model.Finish(RunState.Succeeded);

            _viewModel.Refresh();

            Assert.True(_viewModel.IsFinished);
            Assert.True(_viewModel.IsSucceeded);
            Assert.Equal("Close", _viewModel.ActionButtonText);
            Assert.True(_viewModel.ActionButtonEnabled);
            Assert.Equal(1.0, _viewModel.OverallFraction);
            Assert.Equal("Installation complete — 3 of 3 steps", _viewModel.OverallLabel);
            Assert.All(_viewModel.Steps, row => Assert.Equal(1.0, row.Fraction));
            Assert.True(_viewModel.CanCloseWithoutAsking);
        }

        [Fact]
        public void AFailureIsShownByNameAndFlagged()
        {
            _model.BeginPhase("Converting meshes", 10, null);
            _model.Fail(["Converting meshes"]);
            _model.Finish(RunState.Failed);

            _viewModel.Refresh();

            Assert.True(_viewModel.IsFailed);
            Assert.False(_viewModel.IsSucceeded);
            Assert.Equal("Installation failed — Converting meshes", _viewModel.OverallLabel);
            Assert.True(_viewModel.Steps[1].IsFailed);
            Assert.Equal("Close", _viewModel.ActionButtonText);
        }

        [Fact]
        public void ACancelledRunIsFlaggedAndItsStepsAreMarkedCancelled()
        {
            _model.BeginPhase("Extracting", 0, null);
            _model.Finish(RunState.Cancelled);

            _viewModel.Refresh();

            Assert.True(_viewModel.IsCancelled);
            Assert.All(_viewModel.Steps, row => Assert.True(row.IsCancelled));
            Assert.Equal("Close", _viewModel.ActionButtonText);
        }

        [Fact]
        public void CancellingRaisesTheRequestOnceAndWaitsForTheRunToEnd()
        {
            _viewModel.Refresh();

            _viewModel.RequestCancel();
            _viewModel.RequestCancel();

            Assert.Equal(1, _cancelRequests);
            Assert.True(_viewModel.CancelRequested);
            Assert.Equal("Cancelling…", _viewModel.ActionButtonText);
            Assert.False(_viewModel.ActionButtonEnabled);
            Assert.True(_viewModel.CanCloseWithoutAsking);

            _model.Finish(RunState.Cancelled);
            _viewModel.Refresh();

            Assert.Equal("Close", _viewModel.ActionButtonText);
            Assert.True(_viewModel.ActionButtonEnabled);
        }

        [Fact]
        public void TheViewModelAlreadyReadsAsCancellingWhenTheCancelCallbackRuns()
        {
            InstallerViewModel? viewModel = null;
            (bool Requested, bool CanClose, bool ButtonEnabled)? seen = null;
            viewModel = new InstallerViewModel(_model.GetSnapshot, _log, () =>
                seen = (viewModel!.CancelRequested, viewModel.CanCloseWithoutAsking, viewModel.ActionButtonEnabled));

            viewModel.RequestCancel();

            Assert.Equal((true, true, false), seen);
        }

        [Fact]
        public void ACancelRequestMadeFromInsideTheCancelCallbackIsIgnored()
        {
            int calls = 0;
            InstallerViewModel? viewModel = null;
            viewModel = new InstallerViewModel(_model.GetSnapshot, _log, () =>
            {
                calls++;
                viewModel!.RequestCancel();
            });

            viewModel.RequestCancel();

            Assert.Equal(1, calls);
        }

        [Fact]
        public void LinesThatTheBufferOverwroteBeforeTheyWereReadAreMarkedAsDropped()
        {
            var smallLog = new LogBuffer(10);
            var viewModel = new InstallerViewModel(_model.GetSnapshot, smallLog, () => { }, @"C:\install\install.log");
            for (int i = 0; i < 25; i++) smallLog.WriteLine($"line {i}", false);

            viewModel.Refresh();
            smallLog.WriteLine("after", false);
            viewModel.Refresh();

            Assert.Equal(12, viewModel.LogLines.Count);
            Assert.Equal("… 15 lines dropped from this view (the log file has them)", viewModel.LogLines[0].Text);
            Assert.False(viewModel.LogLines[0].IsError);
            Assert.EndsWith("line 15", viewModel.LogLines[1].Text, StringComparison.Ordinal);
            Assert.EndsWith("after", viewModel.LogLines[^1].Text, StringComparison.Ordinal);
            Assert.DoesNotContain(viewModel.LogLines.Skip(1), line => line.Text.Contains("dropped", StringComparison.Ordinal));
        }

        [Fact]
        public void ARunningInstallNeedsConfirmationBeforeTheWindowCloses()
        {
            _viewModel.Refresh();

            Assert.False(_viewModel.CanCloseWithoutAsking);
        }

        [Fact]
        public void CancellingAFinishedRunDoesNothing()
        {
            _model.Finish(RunState.Succeeded);
            _viewModel.Refresh();

            _viewModel.RequestCancel();

            Assert.Equal(0, _cancelRequests);
        }

        [Fact]
        public void NewLogLinesAreAppendedOnEachRefreshWithErrorsFlagged()
        {
            _log.WriteLine("Unpacking archive", false);
            _viewModel.Refresh();
            _log.WriteLine("WARNING: something", true);
            _log.WriteLine("more", false);
            _viewModel.Refresh();
            _viewModel.Refresh();

            Assert.Equal(3, _viewModel.LogLines.Count);
            Assert.EndsWith("Unpacking archive", _viewModel.LogLines[0].Text, StringComparison.Ordinal);
            Assert.False(_viewModel.LogLines[0].IsError);
            Assert.True(_viewModel.LogLines[1].IsError);
            Assert.EndsWith("more", _viewModel.LogLines[2].Text, StringComparison.Ordinal);
        }

        [Fact]
        public void TheLogViewKeepsOnlyTheNewestLines()
        {
            for (int i = 0; i < InstallerViewModel.LogCapacity + 50; i++) _log.WriteLine($"line {i}", false);

            _viewModel.Refresh();

            Assert.Equal(InstallerViewModel.LogCapacity, _viewModel.LogLines.Count);
            Assert.EndsWith($"line {InstallerViewModel.LogCapacity + 49}", _viewModel.LogLines[^1].Text, StringComparison.Ordinal);
            Assert.EndsWith("line 50", _viewModel.LogLines[0].Text, StringComparison.Ordinal);
        }

        [Fact]
        public void AnUnchangedSnapshotRaisesNoPropertyChanges()
        {
            _model.BeginPhase("Converting meshes", 100, null);
            _viewModel.Refresh();
            var changed = new List<string?>();
            _viewModel.PropertyChanged += (_, e) => changed.Add(e.PropertyName);
            var rowChanges = new List<string?>();
            _viewModel.Steps[1].PropertyChanged += (_, e) => rowChanges.Add(e.PropertyName);

            _viewModel.Refresh();

            Assert.Empty(changed);
            Assert.Empty(rowChanges);
        }

        [Fact]
        public void AChangedValueRaisesItsPropertyChangedNotification()
        {
            int phase = _model.BeginPhase("Converting meshes", 100, null);
            _viewModel.Refresh();
            var changed = new List<string?>();
            var rowChanged = new List<string?>();
            ((INotifyPropertyChanged)_viewModel).PropertyChanged += (_, e) => changed.Add(e.PropertyName);
            _viewModel.Steps[1].PropertyChanged += (_, e) => rowChanged.Add(e.PropertyName);

            _model.Tick(phase, 10);
            _viewModel.Refresh();

            Assert.Contains(nameof(InstallerViewModel.OverallFraction), changed);
            Assert.Contains(nameof(StepRowViewModel.Fraction), rowChanged);
        }

        [Fact]
        public void NumbersAndDurationsFollowTheCultureTheViewModelWasGiven()
        {
            var german = new InstallerViewModel(_model.GetSnapshot, _log, () => { }, culture: new CultureInfo("de-DE"));
            int meshes = _model.BeginPhase("Converting meshes", 63_000, null);
            _model.Tick(meshes, 12_345);
            _clock.Advance(TimeSpan.FromMilliseconds(2_500));

            german.Refresh();

            Assert.Equal("12.345 / 63.000", german.Steps[1].CountText);
            Assert.Equal("2,5s", german.Steps[1].ElapsedText);
            Assert.Equal("Elapsed 2,5s", german.ElapsedText);
        }

        [Fact]
        public void AStepThatHadNoWorkReadsAsSkippedInTheListOnceTheRunSucceedsAndShowsATimeOnlyIfItStarted()
        {
            int extracting = _model.BeginPhase("Extracting", 0, null);
            _clock.Advance(TimeSpan.FromSeconds(2));
            _model.EndPhase(extracting);
            _model.Finish(RunState.Succeeded);

            _viewModel.Refresh();

            Assert.All(_viewModel.Steps, row => Assert.Equal(ProgressText.Skipped, row.CountText));
            Assert.Equal("2.0s", _viewModel.Steps[0].ElapsedText);
            Assert.Equal("", _viewModel.Steps[1].ElapsedText);
        }

        [Fact]
        public void TheOpenButtonsAreOfferedOnceTheRunEndedWhateverItsOutcome()
        {
            _viewModel.Refresh();
            Assert.False(_viewModel.CanOpenInstallFolder);
            Assert.False(_viewModel.CanOpenLog);

            _model.Finish(RunState.Failed);
            _viewModel.Refresh();

            Assert.True(_viewModel.CanOpenInstallFolder);
            Assert.True(_viewModel.CanOpenLog);
            Assert.Equal(@"C:\install", _viewModel.InstallFolderPath);
            Assert.Equal(@"C:\install\install.log", _viewModel.LogFilePath);
        }

        [Fact]
        public void AViewModelForAnInstallUnderWayIsNotSettingUp()
        {
            Assert.False(_viewModel.IsSettingUp);
            Assert.Null(_viewModel.Setup);
        }

        [Fact]
        public void WhileSettingUpTheViewModelShowsTheSetupAndLetsTheWindowCloseWithoutAsking()
        {
            var setup = new SetupViewModel();
            var viewModel = new InstallerViewModel(setup, CultureInfo.InvariantCulture);

            viewModel.Refresh();
            viewModel.RequestCancel();

            Assert.True(viewModel.IsSettingUp);
            Assert.Same(setup, viewModel.Setup);
            Assert.True(viewModel.CanCloseWithoutAsking);
            Assert.False(viewModel.CancelRequested);
            Assert.Empty(viewModel.Steps);
        }

        [Fact]
        public void BeginningTheInstallSwitchesToTheProgressAndFromThenOnClosingAsksAgain()
        {
            var viewModel = new InstallerViewModel(new SetupViewModel(), CultureInfo.InvariantCulture);
            var changed = new List<string?>();
            viewModel.PropertyChanged += (_, e) => changed.Add(e.PropertyName);

            viewModel.BeginInstall(_model.GetSnapshot, _log, () => _cancelRequests++, @"D:\OpenFO4\install.log");
            viewModel.Refresh();

            Assert.False(viewModel.IsSettingUp);
            Assert.False(viewModel.CanCloseWithoutAsking);
            Assert.Equal(3, viewModel.Steps.Count);
            Assert.Equal(@"D:\OpenFO4", viewModel.InstallFolderPath);
            Assert.Equal(@"Log file: D:\OpenFO4\install.log", viewModel.LogFileText);
            Assert.Contains(nameof(InstallerViewModel.IsSettingUp), changed);
            viewModel.RequestCancel();
            Assert.Equal(1, _cancelRequests);
        }

        private static SetupViewModel StartedSetup(TempDirectory root)
        {
            string game = Path.Combine(root.Path, "Fallout 4");
            Directory.CreateDirectory(Path.Combine(game, "Data"));
            File.WriteAllBytes(Path.Combine(game, "Fallout4.exe"), []);
            File.WriteAllBytes(Path.Combine(game, "Data", "Fallout4.esm"), []);
            var setup = new SetupViewModel(game) { InstallFolder = Path.Combine(root.Path, "install") };
            return setup;
        }

        [Fact]
        public void AfterInstallIsPressedClosingTheWindowAsksBeforeTheInstallBegins()
        {
            using var root = new TempDirectory();
            var setup = StartedSetup(root);
            var viewModel = new InstallerViewModel(setup, CultureInfo.InvariantCulture);
            var changed = new List<string?>();
            viewModel.PropertyChanged += (_, e) => changed.Add(e.PropertyName);
            Assert.True(viewModel.CanCloseWithoutAsking);

            setup.Start();

            Assert.True(setup.HasStarted);
            Assert.False(viewModel.CanCloseWithoutAsking);
            Assert.Contains(nameof(InstallerViewModel.CanCloseWithoutAsking), changed);
        }

        [Fact]
        public void ACancellationBetweenInstallBeingPressedAndTheInstallBeginningCancelsTheInstallAsItBegins()
        {
            using var root = new TempDirectory();
            var setup = StartedSetup(root);
            var viewModel = new InstallerViewModel(setup, CultureInfo.InvariantCulture);
            setup.Start();

            viewModel.RequestCancel();
            Assert.Equal(0, _cancelRequests);
            viewModel.BeginInstall(_model.GetSnapshot, _log, () => _cancelRequests++, Path.Combine(root.Path, "install", "install.log"));

            Assert.Equal(1, _cancelRequests);
            Assert.True(viewModel.CancelRequested);
            Assert.True(viewModel.CanCloseWithoutAsking);
        }

        [Fact]
        public void AnInstallThatBeginsWithoutACancellationIsNotCancelled()
        {
            using var root = new TempDirectory();
            var setup = StartedSetup(root);
            var viewModel = new InstallerViewModel(setup, CultureInfo.InvariantCulture);
            setup.Start();

            viewModel.BeginInstall(_model.GetSnapshot, _log, () => _cancelRequests++, Path.Combine(root.Path, "install", "install.log"));

            Assert.Equal(0, _cancelRequests);
            Assert.False(viewModel.CanCloseWithoutAsking);
        }

        [Fact]
        public void TheInstallCanBeginOnlyOnce()
        {
            var viewModel = new InstallerViewModel(new SetupViewModel(), CultureInfo.InvariantCulture);
            viewModel.BeginInstall(_model.GetSnapshot, _log, () => { }, @"D:\OpenFO4\install.log");

            Assert.Throws<InvalidOperationException>(() => viewModel.BeginInstall(_model.GetSnapshot, _log, () => { }, @"D:\OpenFO4\install.log"));
            Assert.Throws<InvalidOperationException>(() => _viewModel.BeginInstall(_model.GetSnapshot, _log, () => { }, @"D:\x\install.log"));
        }

        [Fact]
        public void WithoutALogFileThereIsNothingToOpen()
        {
            var viewModel = new InstallerViewModel(_model.GetSnapshot, _log, () => { }, culture: CultureInfo.InvariantCulture);
            _model.Finish(RunState.Succeeded);

            viewModel.Refresh();

            Assert.False(viewModel.CanOpenInstallFolder);
            Assert.False(viewModel.CanOpenLog);
        }
    }
}
