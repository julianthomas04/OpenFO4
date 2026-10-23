using System.Runtime.Versioning;
using System.Security;
using Microsoft.Win32;

namespace AssetConverter
{
    /// <summary>The game folder found through Steam, and why sources of candidates gave none; the notes are for the caller to log.</summary>
    internal sealed record SteamGameFolderSearch(string? Folder, IReadOnlyList<string> Notes);

    /// <summary>
    /// Finds the Fallout 4 folder of a Steam install: the Steam root from <paramref name="steamRoot"/>, every library listed in its
    /// <c>libraryfolders.vdf</c> (the Steam root itself included), and in each library the <c>installdir</c> of the Fallout 4 app
    /// manifest. Never throws; whatever cannot be read is reported in the notes.
    /// </summary>
    internal sealed class SteamGameFolderLocator(Func<string?> steamRoot)
    {
        public const string AppId = "377160";

        public static SteamGameFolderLocator FromRegistry() => new(SteamRootFromRegistry);

        public SteamGameFolderSearch Find()
        {
            var notes = new List<string>();
            foreach (string library in Libraries(notes))
            {
                string? candidate = Candidate(library, notes);
                if (candidate == null) continue;
                if (InstallFolderRules.DefaultGameFolder(candidate) is { } folder) return new SteamGameFolderSearch(folder, notes);
                notes.Add($"{candidate} does not hold {InstallFolderRules.GameExecutable}.");
            }
            return new SteamGameFolderSearch(null, notes);
        }

        private IEnumerable<string> Libraries(List<string> notes)
        {
            string? root = ReadSteamRoot(notes);
            if (root == null) return [];
            var libraries = new List<string> { root };
            if (TryReadKeyValues(Path.Combine(root, "steamapps", "libraryfolders.vdf"), notes) is { } document)
            {
                libraries.AddRange(document.Children.Select(entry => entry.Value.Value ?? entry.Value.ValueOf("path")).Where(path => !string.IsNullOrWhiteSpace(path))!);
            }
            return libraries.Distinct(StringComparer.OrdinalIgnoreCase);
        }

        private string? ReadSteamRoot(List<string> notes)
        {
            try
            {
                string? root = steamRoot();
                if (string.IsNullOrWhiteSpace(root)) notes.Add("The Steam folder is not recorded in the registry.");
                else if (!Directory.Exists(root)) notes.Add($"The Steam folder {root} does not exist.");
                else return Path.GetFullPath(root);
            }
            catch (Exception ex) when (ex is IOException or UnauthorizedAccessException or SecurityException or ArgumentException or NotSupportedException)
            {
                notes.Add($"The Steam folder could not be determined: {ex.Message}");
            }
            return null;
        }

        private static string? Candidate(string library, List<string> notes)
        {
            string manifest = Path.Combine(library, "steamapps", $"appmanifest_{AppId}.acf");
            if (!File.Exists(manifest)) return null;
            string? installDir = TryReadKeyValues(manifest, notes)?.ValueOf("installdir");
            if (string.IsNullOrWhiteSpace(installDir))
            {
                notes.Add($"{manifest} names no install directory.");
                return null;
            }
            return Path.Combine(library, "steamapps", "common", installDir);
        }

        private static KeyValuesNode? TryReadKeyValues(string path, List<string> notes)
        {
            try
            {
                return KeyValuesText.Parse(File.ReadAllText(path));
            }
            catch (Exception ex) when (ex is FileNotFoundException or DirectoryNotFoundException)
            {
                notes.Add($"{path} does not exist.");
            }
            catch (Exception ex) when (ex is IOException or UnauthorizedAccessException)
            {
                notes.Add($"{path} cannot be read: {ex.Message}");
            }
            catch (KeyValuesFormatException ex)
            {
                notes.Add($"{path} is not valid: {ex.Message}");
            }
            return null;
        }

        private static string? SteamRootFromRegistry() =>
            !OperatingSystem.IsWindows() ? null :
            ReadRegistryString(Registry.CurrentUser, @"Software\Valve\Steam", "SteamPath")
            ?? ReadRegistryString(Registry.LocalMachine, @"SOFTWARE\WOW6432Node\Valve\Steam", "InstallPath");

        [SupportedOSPlatform("windows")]
        private static string? ReadRegistryString(RegistryKey hive, string subKey, string name)
        {
            using RegistryKey? key = hive.OpenSubKey(subKey);
            return key?.GetValue(name) as string;
        }
    }
}
