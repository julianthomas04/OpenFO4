using AssetConverter.Pipeline;
namespace AssetConverter.InstallerUi
{
    /// <summary>The folders and options the user confirmed on the setup view; all folders are full paths, <paramref name="InstallFolder"/> is the install root itself and <paramref name="SettingsFolder"/> (null when the setup does not ask for it) holds the INIs the game wrote.</summary>
    public sealed record SetupChoice(string GameFolder, string InstallFolder, bool KeepStaging, string? SettingsFolder = null);

    /// <summary>Lets the user pick a folder, for the Browse buttons.</summary>
    public interface IFolderPicker
    {
        /// <summary>The folder the user picked, or null when they dismissed the dialog.</summary>
        Task<string?> PickFolderAsync(string title, string? startFolder);
    }

    /// <summary>
    /// The setup view: the game folder, the folder to install in (OpenFO4 gets a folder of its own inside it) and whether to keep the temporary files. The install does not start before
    /// <see cref="Start"/>, which is possible only while both folders are valid; <see cref="Completed"/> then yields what was chosen,
    /// or null when the user cancelled or closed the window.
    /// </summary>
    public sealed class SetupViewModel : ObservableObject
    {
        private readonly TaskCompletionSource<SetupChoice?> _completed = new(TaskCreationOptions.RunContinuationsAsynchronously);
        private string _gameFolder;
        private string _installFolder = "";
        private bool _keepStaging;
        private string _gameFolderError = "";
        private string _gameIniError = "";
        private string _installFolderError = "";
        private readonly bool _asksForSettingsFolder;
        private string _settingsFolder;
        private string _installRoot = "";
        private string _startError = "";
        private bool _hasStarted;

        /// <param name="gameFolder">The game folder to start with; empty for none.</param>
        /// <param name="settingsFolder">The folder to start with for the INIs the game writes on its first launch (Fallout4.ini, Fallout4Prefs.ini); null leaves the settings folder out of the setup and skips the check for the game's INIs.</param>
        public SetupViewModel(string gameFolder = "", bool keepStaging = false, string? settingsFolder = null)
        {
            ArgumentNullException.ThrowIfNull(gameFolder);
            _gameFolder = gameFolder;
            _asksForSettingsFolder = settingsFolder != null;
            _settingsFolder = settingsFolder ?? "";
            _keepStaging = keepStaging;
            ValidateGameFolder();
            ValidateInstallFolder();
        }

        public Task<SetupChoice?> Completed => _completed.Task;

        /// <summary>True once <see cref="Start"/> went through: the install is about to begin and the setup can no longer be cancelled.</summary>
        public bool HasStarted => _hasStarted;

        public string GameFolder
        {
            get => _gameFolder;
            set
            {
                if (_gameFolder == value) return;
                _gameFolder = value ?? "";
                _startError = "";
                RaiseChanged(nameof(GameFolder));
                ValidateGameFolder();
                ValidateInstallFolder();
                RaiseValidationChanged();
            }
        }

        public string InstallFolder
        {
            get => _installFolder;
            set
            {
                if (_installFolder == value) return;
                _installFolder = value ?? "";
                _startError = "";
                RaiseChanged(nameof(InstallFolder));
                ValidateInstallFolder();
                RaiseValidationChanged();
            }
        }

        public bool AsksForSettingsFolder => _asksForSettingsFolder;

        public string SettingsFolder
        {
            get => _settingsFolder;
            set
            {
                if (_settingsFolder == value) return;
                _settingsFolder = value ?? "";
                _startError = "";
                RaiseChanged(nameof(SettingsFolder));
                ValidateGameFolder();
                RaiseValidationChanged();
            }
        }

        public bool KeepStaging
        {
            get => _keepStaging;
            set => SetProperty(ref _keepStaging, value);
        }

        /// <summary>Why the game folder is not acceptable; checked once per edit (and when starting), not per read.</summary>
        public string GameFolderError => _gameFolderError;

        /// <summary>Why the game's INI files are missing even though the game folder is fine; empty when they exist.</summary>
        public string GameIniError => _gameIniError;

        /// <summary>Why the install folder is not acceptable; checked once per edit (and when starting), not per read.</summary>
        public string InstallFolderError => _installFolderError;

        /// <summary>The folder the install will go into, shown once the chosen folder is acceptable; empty until then.</summary>
        public string InstallRoot => _installRoot;

        public string InstallRootText => _installRoot.Length > 0 ? $"OpenFO4 will be installed into {_installRoot}" : "";

        /// <summary>Why pressing Install could not start the install even though both folders looked fine; empty until then and after the next edit.</summary>
        public string StartError => _startError;

        public bool CanEdit => !_completed.Task.IsCompleted;

        public bool CanStart => CanEdit && GameFolderError.Length == 0 && GameIniError.Length == 0 && InstallFolderError.Length == 0;

        public Task BrowseGameFolderAsync(IFolderPicker picker) =>
            BrowseAsync(picker, "Select the Fallout 4 folder", _gameFolder, picked => GameFolder = picked);

        public Task BrowseSettingsFolderAsync(IFolderPicker picker) =>
            BrowseAsync(picker, "Select the folder holding Fallout4Prefs.ini", _settingsFolder, picked => SettingsFolder = picked);

        public Task BrowseInstallFolderAsync(IFolderPicker picker) =>
            BrowseAsync(picker, "Select the folder to install OpenFO4 in", _installFolder, picked => InstallFolder = picked);

        /// <summary>
        /// Confirms the choice and ends the setup, after checking the folders again and creating the install folder; does nothing while a
        /// folder is invalid. When the install folder cannot be created the setup stays open and <see cref="StartError"/> says why.
        /// </summary>
        public void Start()
        {
            if (!CanEdit) return;
            ValidateGameFolder();
            ValidateInstallFolder();
            _startError = "";
            if (CanStart)
            {
                var choice = new SetupChoice(InstallFolderRules.FullPath(_gameFolder), InstallFolderRules.FullPath(InstallFolderRules.InstallRoot(_installFolder)), _keepStaging,
                    _asksForSettingsFolder ? InstallFolderRules.FullPath(_settingsFolder) : null);
                _startError = InstallFolderPreparation.Prepare(new InstallLocations(choice.GameFolder, choice.InstallFolder, choice.SettingsFolder)) ?? "";
                if (_startError.Length == 0 && _completed.TrySetResult(choice)) _hasStarted = true;
            }
            RaiseValidationChanged();
        }

        /// <summary>Checks the folders and the game's INI files again, for when the user fixed the problem outside the installer.</summary>
        public void Revalidate()
        {
            if (!CanEdit) return;
            ValidateGameFolder();
            ValidateInstallFolder();
            RaiseValidationChanged();
        }

        /// <summary>Ends the setup without a choice; does nothing once <see cref="Start"/> went through.</summary>
        public void Cancel()
        {
            if (_completed.TrySetResult(null)) RaiseValidationChanged();
        }

        private async Task BrowseAsync(IFolderPicker picker, string title, string current, Action<string> apply)
        {
            ArgumentNullException.ThrowIfNull(picker);
            if (!CanEdit) return;
            string? picked = await picker.PickFolderAsync(title, StartFolderFor(current)).ConfigureAwait(true);
            if (picked != null && CanEdit) apply(picked);
        }

        private static string? StartFolderFor(string current)
        {
            string path = InstallFolderRules.Normalize(current);
            return path.Length > 0 && Path.IsPathFullyQualified(path) ? path : null;
        }

        private void ValidateGameFolder()
        {
            _gameFolderError = InstallFolderRules.GameFolderError(_gameFolder) ?? "";
            _gameIniError = _gameFolderError.Length == 0 && _asksForSettingsFolder ? GameIniErrorFor(InstallFolderRules.FullPath(_gameFolder)) : "";
        }

        private string GameIniErrorFor(string gameFolder)
        {
            string settings = InstallFolderRules.Normalize(_settingsFolder);
            if (settings.Length == 0) return "Choose the Fallout 4 settings folder, the folder that holds Fallout4Prefs.ini.";
            if (!Path.IsPathFullyQualified(settings)) return @"Enter a full path, such as C:\Users\you\Documents\My Games\Fallout4.";
            return GameIniFiles.MissingError(gameFolder, InstallFolderRules.FullPath(settings)) ?? "";
        }

        private void ValidateInstallFolder()
        {
            _installFolderError = InstallFolderRules.ChosenFolderError(_installFolder, _gameFolder) ?? "";
            _installRoot = _installFolderError.Length == 0 ? InstallFolderRules.FullPath(InstallFolderRules.InstallRoot(_installFolder)) : "";
        }

        private void RaiseValidationChanged()
        {
            RaiseChanged(nameof(HasStarted));
            RaiseChanged(nameof(GameFolderError));
            RaiseChanged(nameof(GameIniError));
            RaiseChanged(nameof(InstallFolderError));
            RaiseChanged(nameof(InstallRoot));
            RaiseChanged(nameof(InstallRootText));
            RaiseChanged(nameof(StartError));
            RaiseChanged(nameof(CanEdit));
            RaiseChanged(nameof(CanStart));
        }
    }
}
