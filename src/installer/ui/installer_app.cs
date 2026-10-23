using Avalonia;
using Avalonia.Controls.ApplicationLifetimes;
using Avalonia.Styling;
using Avalonia.Themes.Fluent;

namespace AssetConverter.InstallerUi
{
    /// <summary>The Avalonia application hosting the installer window; the theme follows the system light/dark setting.</summary>
    internal sealed class InstallerApp : Application
    {
        private readonly InstallerViewModel _viewModel;
        private readonly Action _windowOpened;

        public InstallerApp(InstallerViewModel viewModel, Action windowOpened)
        {
            ArgumentNullException.ThrowIfNull(viewModel);
            ArgumentNullException.ThrowIfNull(windowOpened);
            _viewModel = viewModel;
            _windowOpened = windowOpened;
        }

        public override void Initialize()
        {
            Styles.Add(new FluentTheme());
            RequestedThemeVariant = ThemeVariant.Default;
        }

        public override void OnFrameworkInitializationCompleted()
        {
            if (ApplicationLifetime is IClassicDesktopStyleApplicationLifetime desktop)
            {
                var window = new InstallerWindow { DataContext = _viewModel };
                window.Opened += (_, _) => _windowOpened();
                desktop.MainWindow = window;
            }
            base.OnFrameworkInitializationCompleted();
        }
    }
}
