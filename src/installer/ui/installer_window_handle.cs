namespace AssetConverter.InstallerUi
{
    /// <summary>The running installer window as the session sees it.</summary>
    internal interface IInstallerWindow
    {
        /// <summary>Completes when the window and its UI thread are gone.</summary>
        Task Closed { get; }

        /// <summary>The error that took the UI thread down after it had started, if any.</summary>
        Exception? Fault { get; }

        /// <summary>
        /// Runs <paramref name="action"/> on the window's UI thread and returns once it has run: the view model and the window's controls
        /// may be touched from that thread only. Throws what the action throws, or <see cref="InstallerWindowException"/> when the window
        /// is gone before it could run.
        /// </summary>
        void RunOnUiThread(Action action);

        /// <summary>Asks the window to shut down, for a run that ends without the user closing it.</summary>
        void Shutdown();
    }
}
