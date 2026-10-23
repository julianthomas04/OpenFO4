using AssetConverter.Logging;
using Avalonia.Controls;
using Avalonia.Interactivity;

namespace AssetConverter.InstallerUi
{
    /// <summary>The setup view; its <see cref="StyledElement.DataContext"/> must be a <see cref="SetupViewModel"/>.</summary>
    public partial class SetupView : UserControl
    {
        public SetupView()
        {
            InitializeComponent();
            AttachedToVisualTree += (_, _) =>
            {
                if (TopLevel.GetTopLevel(this) is Window window) window.Activated += OnWindowActivated;
            };
            DetachedFromVisualTree += (_, e) =>
            {
                if (e.Root is Window window) window.Activated -= OnWindowActivated;
            };
        }

        private void OnWindowActivated(object? sender, EventArgs e) => (DataContext as SetupViewModel)?.Revalidate();

        private SetupViewModel ViewModel =>
            DataContext as SetupViewModel ?? throw new InvalidOperationException("The setup view needs a SetupViewModel as its DataContext.");

        private IFolderPicker Picker =>
            new StorageFolderPicker(TopLevel.GetTopLevel(this) ?? throw new InvalidOperationException("The setup view is not shown in a window."));

        private void OnBrowseGameFolderClick(object? sender, RoutedEventArgs e) =>
            TaskFaultReporter.Observe(ViewModel.BrowseGameFolderAsync(Picker), "choosing the Fallout 4 folder");

        private void OnBrowseSettingsFolderClick(object? sender, RoutedEventArgs e) =>
            TaskFaultReporter.Observe(ViewModel.BrowseSettingsFolderAsync(Picker), "choosing the Fallout 4 settings folder");

        private void OnBrowseInstallFolderClick(object? sender, RoutedEventArgs e) =>
            TaskFaultReporter.Observe(ViewModel.BrowseInstallFolderAsync(Picker), "choosing the install folder");

        private void OnStartClick(object? sender, RoutedEventArgs e) => ViewModel.Start();

        private void OnCancelClick(object? sender, RoutedEventArgs e)
        {
            ViewModel.Cancel();
            (TopLevel.GetTopLevel(this) as Window)?.Close();
        }
    }
}
