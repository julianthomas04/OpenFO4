using System.Diagnostics;

namespace AssetConverter.InstallerUi
{
    /// <summary>Starts the installed game, for the Play button.</summary>
    public interface IGameLauncher
    {
        /// <exception cref="System.ComponentModel.Win32Exception">The system could not start the program.</exception>
        void Launch(string executable, string workingDirectory);
    }

    /// <summary>Starts the game as a process of its own that outlives the installer, without a console window and without the installer's engine settings.</summary>
    internal sealed class ProcessGameLauncher : IGameLauncher
    {
        private static readonly string[] EngineLocationVariables = ["OPENFO4_INSTALL_DIR", "OPENFO4_ASSET_ROOT"];

        public void Launch(string executable, string workingDirectory)
        {
            using var process = Process.Start(StartInfo(executable, workingDirectory))
                ?? throw new InvalidOperationException($"The system did not start '{executable}'.");
        }

        /// <summary>The game must find its assets beside itself, so the variables that point the engine somewhere else are not passed on from the installer's environment.</summary>
        internal static ProcessStartInfo StartInfo(string executable, string workingDirectory)
        {
            var startInfo = new ProcessStartInfo(executable) { WorkingDirectory = workingDirectory, UseShellExecute = false, CreateNoWindow = true };
            foreach (string variable in EngineLocationVariables) startInfo.Environment.Remove(variable);
            return startInfo;
        }
    }
}
