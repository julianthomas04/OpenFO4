using System.ComponentModel;
using AssetConverter.Logging;
using Avalonia;
using Avalonia.Controls;
using Avalonia.Interactivity;
using Avalonia.Threading;

namespace AssetConverter.InstallerUi
{
    /// <summary>The installer window; its <see cref="StyledElement.DataContext"/> must be an <see cref="InstallerViewModel"/>.</summary>
    public partial class InstallerWindow : Window
    {
        private static readonly TimeSpan RefreshInterval = TimeSpan.FromMilliseconds(100);

        private readonly DispatcherTimer _refreshTimer;
        private bool _followLog = true;
        private Action? _withdrawConfirmDialog;
        private bool _closeWindowWhenConfirmDialogEnds;
        private PathOpener? _pathOpener;

        public InstallerWindow()
        {
            InitializeComponent();
            Title = $"{Title} {InstallerVersion.Current}";
            _refreshTimer = new DispatcherTimer(RefreshInterval, DispatcherPriority.Background, (_, _) => ViewModel.Refresh());
            LogScroll.ScrollChanged += OnLogScrollChanged;
            StepScroll.ScrollChanged += OnStepScrollChanged;
        }

        private void OnStepScrollChanged(object? sender, ScrollChangedEventArgs e) =>
            StepHeader.Margin = new Thickness(LayoutConverters.StepRowInset, 0, LayoutConverters.StepRowInset + StepScroll.Bounds.Width - StepScroll.Viewport.Width, 0);

        private InstallerViewModel ViewModel =>
            DataContext as InstallerViewModel ?? throw new InvalidOperationException("The installer window needs an InstallerViewModel as its DataContext.");

        protected override void OnOpened(EventArgs e)
        {
            base.OnOpened(e);
            ViewModel.Refresh();
            ViewModel.PropertyChanged += OnViewModelPropertyChanged;
            _refreshTimer.Start();
        }

        private void OnViewModelPropertyChanged(object? sender, PropertyChangedEventArgs e)
        {
            if (e.PropertyName == nameof(InstallerViewModel.CanCloseWithoutAsking) && ViewModel.CanCloseWithoutAsking)
                _withdrawConfirmDialog?.Invoke();
        }

        protected override void OnClosing(WindowClosingEventArgs e)
        {
            base.OnClosing(e);
            if (ViewModel.CanCloseWithoutAsking)
            {
                _closeWindowWhenConfirmDialogEnds = false;
                _withdrawConfirmDialog?.Invoke();
                return;
            }
            e.Cancel = true;
            ConfirmCancel(fromCloseRequest: true);
        }

        protected override void OnClosed(EventArgs e)
        {
            _refreshTimer.Stop();
            ViewModel.PropertyChanged -= OnViewModelPropertyChanged;
            ViewModel.Setup?.Cancel();
            base.OnClosed(e);
        }

        private void OnActionButtonClick(object? sender, RoutedEventArgs e)
        {
            if (ViewModel.IsFinished) Close();
            else ConfirmCancel(fromCloseRequest: false);
        }

        private void OnPlayClick(object? sender, RoutedEventArgs e)
        {
            if (ViewModel.Play()) Close();
        }

        private PathOpener PathOpener => _pathOpener ??= new PathOpener(Launcher, Console.Error.WriteLine);

        private void OnOpenInstallFolderClick(object? sender, RoutedEventArgs e) =>
            TaskFaultReporter.Observe(PathOpener.OpenFolderAsync(ViewModel.InstallFolderPath!), "opening the install folder");

        private void OnOpenLogClick(object? sender, RoutedEventArgs e) =>
            TaskFaultReporter.Observe(PathOpener.OpenFileAsync(ViewModel.LogFilePath!), "opening the log file");

        private void ConfirmCancel(bool fromCloseRequest)
        {
            _closeWindowWhenConfirmDialogEnds |= fromCloseRequest;
            if (_withdrawConfirmDialog != null) return;
            TaskFaultReporter.Observe(ConfirmCancelAsync(), "the cancel confirmation");
        }

        private async Task ConfirmCancelAsync()
        {
            using var withdrawal = new CancellationTokenSource();
            _withdrawConfirmDialog = withdrawal.Cancel;
            bool confirmed;
            try
            {
                confirmed = await ConfirmDialog.ShowAsync(this, "Cancel installation",
                    "Cancel the installation? Work done so far is kept; the next run resumes where possible.",
                    confirmText: "Cancel installation", declineText: "Keep installing", withdrawn: withdrawal.Token);
            }
            finally
            {
                _withdrawConfirmDialog = null;
            }

            bool closeWindow = _closeWindowWhenConfirmDialogEnds && !confirmed && ViewModel.CanCloseWithoutAsking;
            _closeWindowWhenConfirmDialogEnds = false;
            if (confirmed) ViewModel.RequestCancel();
            else if (closeWindow) Close();
        }

        // Stay pinned to the newest line while following; scrolling up by hand stops following until the user returns to the bottom.
        private void OnLogScrollChanged(object? sender, ScrollChangedEventArgs e)
        {
            LogItems.Margin = new Thickness(0, 0, 0, LayoutConverters.LogEndPaddingFor(LogScroll.Viewport.Height));
            if (e.ExtentDelta.Y != 0)
            {
                if (_followLog) LogScroll.ScrollToEnd();
                return;
            }
            _followLog = LogScroll.Offset.Y + LogScroll.Viewport.Height >= LogScroll.Extent.Height - 2;
        }
    }
}
