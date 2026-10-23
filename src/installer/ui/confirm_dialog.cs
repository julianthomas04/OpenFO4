using Avalonia;
using Avalonia.Controls;
using Avalonia.Input;
using Avalonia.Layout;
using Avalonia.Threading;

namespace AssetConverter.InstallerUi
{
    /// <summary>A small modal question with a confirm and a decline button; Escape declines. Cancelling the token withdraws the question, which counts as declined.</summary>
    internal static class ConfirmDialog
    {
        public static async Task<bool> ShowAsync(Window owner, string title, string message, string confirmText, string declineText, CancellationToken withdrawn = default)
        {
            var dialog = new Window
            {
                Title = title,
                SizeToContent = SizeToContent.WidthAndHeight,
                CanResize = false,
                WindowStartupLocation = WindowStartupLocation.CenterOwner,
                ShowInTaskbar = false,
            };

            var confirm = new Button { Content = confirmText, MinWidth = 140 };
            var decline = new Button { Content = declineText, MinWidth = 140, IsDefault = true };
            confirm.Click += (_, _) => dialog.Close(true);
            decline.Click += (_, _) => dialog.Close(false);
            dialog.KeyDown += (_, e) =>
            {
                if (e.Key == Key.Escape) dialog.Close(false);
            };

            dialog.Content = new StackPanel
            {
                Margin = new Thickness(20),
                Spacing = 18,
                Children =
                {
                    new TextBlock { Text = message, TextWrapping = Avalonia.Media.TextWrapping.Wrap, MaxWidth = 380 },
                    new StackPanel
                    {
                        Orientation = Orientation.Horizontal,
                        HorizontalAlignment = HorizontalAlignment.Right,
                        Spacing = 10,
                        Children = { confirm, decline },
                    },
                },
            };

            using var withdrawal = withdrawn.Register(() => CloseOnUiThread(dialog));
            return await dialog.ShowDialog<bool>(owner);
        }

        private static void CloseOnUiThread(Window dialog)
        {
            if (Dispatcher.UIThread.CheckAccess()) dialog.Close(false);
            else Dispatcher.UIThread.Post(() => dialog.Close(false));
        }
    }
}
