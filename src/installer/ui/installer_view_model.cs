using System.Collections.ObjectModel;
using System.ComponentModel;
using System.Globalization;
using AssetConverter.Logging;
using AssetConverter.Progress;

namespace AssetConverter.InstallerUi
{
    /// <summary>
    /// What the installer window shows. <see cref="Refresh"/> pulls one progress snapshot and the new log lines; the window calls it on a timer.
    /// Numbers, durations and percentages are formatted with the culture given at construction (the current culture by default); the
    /// log lines are shown as the converters wrote them, in whatever culture each of them used.
    /// </summary>
    public sealed class InstallerViewModel : ObservableObject
    {
        public const int LogCapacity = 10_000;

        private readonly ProgressFormatter _format;
        private Func<ProgressSnapshot>? _getSnapshot;
        private LogBuffer? _log;
        private Action? _requestCancel;
        private string? _logFilePath;
        private string? _installFolderPath;
        private string _logFileText = "";
        private long _lastLogSequence;
        private bool _cancelRequested;
        private readonly IGameLauncher _gameLauncher;
        private bool _canPlay;
        private bool _playDecided;
        private string _playError = "";

        private double _overallFraction;
        private string _overallPercentText = "0%";
        private string _overallLabel = "";
        private string _elapsedText = "";
        private bool _isFinished;
        private RunState _runState = RunState.Running;
        private string _actionButtonText = "Cancel";

        /// <summary>A view model whose install is under way. <paramref name="logFilePath"/> is the install log; the install folder is the folder containing it.</summary>
        public InstallerViewModel(Func<ProgressSnapshot> getSnapshot, LogBuffer log, Action requestCancel, string? logFilePath = null, CultureInfo? culture = null, IGameLauncher? gameLauncher = null)
            : this(culture, gameLauncher)
        {
            AttachInstall(getSnapshot, log, requestCancel, logFilePath);
        }

        /// <summary>A view model that shows <paramref name="setup"/> until <see cref="BeginInstall"/> hands it the install to show.</summary>
        public InstallerViewModel(SetupViewModel setup, CultureInfo? culture = null, IGameLauncher? gameLauncher = null)
            : this(culture, gameLauncher)
        {
            ArgumentNullException.ThrowIfNull(setup);
            Setup = setup;
            IsSettingUp = true;
            setup.PropertyChanged += (_, e) =>
            {
                if (e.PropertyName == nameof(SetupViewModel.HasStarted)) RaiseChanged(nameof(CanCloseWithoutAsking));
            };
        }

        private InstallerViewModel(CultureInfo? culture, IGameLauncher? gameLauncher)
        {
            _gameLauncher = gameLauncher ?? new ProcessGameLauncher();
            _format = new ProgressFormatter(culture ?? CultureInfo.CurrentCulture);
            LogLines = new LogLineCollection(LogCapacity);
        }

        /// <summary>The setup the window starts on; null for a view model created for an install already under way.</summary>
        public SetupViewModel? Setup { get; }

        /// <summary>True until <see cref="BeginInstall"/>: the window shows the setup instead of the progress.</summary>
        public bool IsSettingUp { get; private set; }

        /// <summary>
        /// Switches from the setup to the progress of the install that now runs, and cancels that install at once if the user asked for it
        /// between pressing Install and this call. Like everything else of the view model it belongs to the window's UI thread.
        /// </summary>
        public void BeginInstall(Func<ProgressSnapshot> getSnapshot, LogBuffer log, Action requestCancel, string logFilePath)
        {
            if (!IsSettingUp) throw new InvalidOperationException("The install has begun already.");
            AttachInstall(getSnapshot, log, requestCancel, logFilePath);
            IsSettingUp = false;
            RaiseChanged(nameof(IsSettingUp));
            RaiseChanged(nameof(CanCloseWithoutAsking));
            if (_cancelRequested) requestCancel();
        }

        private void AttachInstall(Func<ProgressSnapshot> getSnapshot, LogBuffer log, Action requestCancel, string? logFilePath)
        {
            ArgumentNullException.ThrowIfNull(getSnapshot);
            ArgumentNullException.ThrowIfNull(log);
            ArgumentNullException.ThrowIfNull(requestCancel);
            _getSnapshot = getSnapshot;
            _log = log;
            _requestCancel = requestCancel;
            LogFilePath = logFilePath;
            InstallFolderPath = logFilePath != null ? Path.GetDirectoryName(Path.GetFullPath(logFilePath)) : null;
            LogFileText = logFilePath != null ? $"Log file: {logFilePath}" : "";
        }

        public ObservableCollection<StepRowViewModel> Steps { get; } = new();

        public LogLineCollection LogLines { get; }

        public string LogFileText
        {
            get => _logFileText;
            private set => SetProperty(ref _logFileText, value);
        }

        public string? LogFilePath
        {
            get => _logFilePath;
            private set => SetProperty(ref _logFilePath, value);
        }

        public string? InstallFolderPath
        {
            get => _installFolderPath;
            private set => SetProperty(ref _installFolderPath, value);
        }

        /// <summary>The game executable inside the install folder.</summary>
        public string? GameExecutablePath => InstallFolderPath == null ? null : Path.Combine(InstallFolderPath, GameFiles.ExecutableName);

        /// <summary>True once the install succeeded and left a game executable in the install folder; decided when the run ends.</summary>
        public bool CanPlay
        {
            get => _canPlay;
            private set => SetProperty(ref _canPlay, value);
        }

        /// <summary>Why the last attempt to start the game failed; empty when it did not.</summary>
        public string PlayError
        {
            get => _playError;
            private set => SetProperty(ref _playError, value);
        }

        /// <summary>Starts the installed game with the install folder as its working directory; returns true when it started, and the window should close.</summary>
        public bool Play()
        {
            if (!CanPlay) return false;
            try
            {
                _gameLauncher.Launch(GameExecutablePath!, InstallFolderPath!);
                PlayError = "";
                return true;
            }
            catch (Exception ex) when (ex is Win32Exception or IOException or InvalidOperationException or UnauthorizedAccessException)
            {
                PlayError = $"The game could not be started: {ex.Message}";
                return false;
            }
        }

        public bool CanOpenInstallFolder => IsFinished && InstallFolderPath != null;

        public bool CanOpenLog => IsFinished && LogFilePath != null;

        public double OverallFraction
        {
            get => _overallFraction;
            private set => SetProperty(ref _overallFraction, value);
        }

        public string OverallPercentText
        {
            get => _overallPercentText;
            private set => SetProperty(ref _overallPercentText, value);
        }

        public string OverallLabel
        {
            get => _overallLabel;
            private set => SetProperty(ref _overallLabel, value);
        }

        public string ElapsedText
        {
            get => _elapsedText;
            private set => SetProperty(ref _elapsedText, value);
        }

        public bool IsFinished
        {
            get => _isFinished;
            private set => SetProperty(ref _isFinished, value);
        }

        public bool IsSucceeded => _runState == RunState.Succeeded;

        public bool IsFailed => _runState == RunState.Failed;

        public bool IsCancelled => _runState == RunState.Cancelled;

        public bool CancelRequested => _cancelRequested;

        public string ActionButtonText
        {
            get => _actionButtonText;
            private set => SetProperty(ref _actionButtonText, value);
        }

        public bool ActionButtonEnabled => IsFinished || !_cancelRequested;

        /// <summary>True when closing the window loses nothing: the setup is still open, or the run is over or already being cancelled.</summary>
        public bool CanCloseWithoutAsking => (IsSettingUp && Setup?.HasStarted != true) || IsFinished || _cancelRequested;

        /// <summary>Cancels the run. Between pressing Install and the install beginning there is nothing to cancel yet: the cancellation is applied when it begins.</summary>
        public void RequestCancel()
        {
            if (IsFinished || _cancelRequested || (IsSettingUp && Setup?.HasStarted != true)) return;
            _cancelRequested = true;
            _requestCancel?.Invoke();
            RaiseChanged(nameof(CancelRequested));
            RaiseChanged(nameof(ActionButtonEnabled));
            RaiseChanged(nameof(CanCloseWithoutAsking));
            ActionButtonText = ActionButtonTextFor(IsFinished);
        }

        public void Refresh()
        {
            if (IsSettingUp) return;
            ApplySnapshot(_getSnapshot!());
            ApplyLog();
        }

        private void ApplySnapshot(ProgressSnapshot snapshot)
        {
            OverallFraction = snapshot.OverallFraction;
            OverallPercentText = _format.Percent(snapshot.OverallFraction);
            OverallLabel = ProgressText.Overall(snapshot);
            ElapsedText = $"Elapsed {_format.Duration(snapshot.Elapsed)}";
            SyncSteps(snapshot);
            ApplyRunState(snapshot);
        }

        private void SyncSteps(ProgressSnapshot snapshot)
        {
            while (Steps.Count < snapshot.Steps.Count) Steps.Add(new StepRowViewModel());
            for (int i = 0; i < snapshot.Steps.Count; i++) Steps[i].Update(snapshot.Steps[i], _format);
        }

        private void ApplyRunState(ProgressSnapshot snapshot)
        {
            if (_runState != snapshot.State)
            {
                _runState = snapshot.State;
                RaiseChanged(nameof(IsSucceeded));
                RaiseChanged(nameof(IsFailed));
                RaiseChanged(nameof(IsCancelled));
            }
            if (IsFinished != snapshot.IsFinished)
            {
                IsFinished = snapshot.IsFinished;
                RaiseChanged(nameof(ActionButtonEnabled));
                RaiseChanged(nameof(CanCloseWithoutAsking));
                RaiseChanged(nameof(CanOpenInstallFolder));
                RaiseChanged(nameof(CanOpenLog));
            }
            if (snapshot.IsFinished && !_playDecided)
            {
                _playDecided = true;
                CanPlay = snapshot.State == RunState.Succeeded && GameExecutablePath != null && File.Exists(GameExecutablePath);
            }
            ActionButtonText = ActionButtonTextFor(snapshot.IsFinished);
        }

        private string ActionButtonTextFor(bool finished) => finished ? "Close" : _cancelRequested ? "Cancelling…" : "Cancel";

        private void ApplyLog()
        {
            var read = _log!.ReadAfter(_lastLogSequence);
            _lastLogSequence = read.LastSequence;
            var lines = read.Lines.Select(line => new LogLineViewModel(line.Format(), line.IsError)).ToList();
            if (read.Missed > 0) lines.Insert(0, new LogLineViewModel(DroppedLinesMarker(read.Missed), IsError: false));
            LogLines.Append(lines);
        }

        private string DroppedLinesMarker(long missed) =>
            $"… {_format.Number(missed)} lines dropped from this view{(LogFileText.Length > 0 ? " (the log file has them)" : "")}";
    }
}
