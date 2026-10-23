using System.ComponentModel;
using Avalonia.Platform.Storage;

namespace AssetConverter.InstallerUi
{
    /// <summary>Opens a folder or file in the platform's default application; every failure is reported through <c>reportError</c>.</summary>
    internal sealed class PathOpener(ILauncher launcher, Action<string> reportError)
    {
        public Task<bool> OpenFolderAsync(string path) =>
            OpenAsync(path, "folder", Directory.Exists, () => launcher.LaunchDirectoryInfoAsync(new DirectoryInfo(path)));

        public Task<bool> OpenFileAsync(string path) =>
            OpenAsync(path, "file", File.Exists, () => launcher.LaunchFileInfoAsync(new FileInfo(path)));

        private async Task<bool> OpenAsync(string path, string kind, Func<string, bool> exists, Func<Task<bool>> launch)
        {
            if (!exists(path))
            {
                reportError($"ERROR: cannot open the {kind} {path}: it does not exist.");
                return false;
            }
            try
            {
                if (await launch()) return true;
                reportError($"ERROR: the system could not open the {kind} {path}.");
            }
            catch (Exception ex) when (ex is Win32Exception or IOException or UnauthorizedAccessException or InvalidOperationException or PlatformNotSupportedException)
            {
                reportError($"ERROR: opening the {kind} {path} failed: {ex}");
            }
            return false;
        }
    }
}
