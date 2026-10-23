using System.Diagnostics.CodeAnalysis;
using AssetConverter.Pipeline;

namespace AssetConverter
{
    /// <summary>
    /// What a folder must be to serve as the Fallout 4 game folder or as the install folder, and which game folder to suggest.
    /// The check for the game folder asks for exactly what the install reads from it directly; individual archives are checked when
    /// the install reaches them.
    /// </summary>
    internal static class InstallFolderRules
    {
        public const string SteamGameFolder = @"C:\Program Files (x86)\Steam\steamapps\common\Fallout 4";
        public const string GameExecutable = "Fallout4.exe";

        /// <summary>The folder the install creates inside the folder the user chooses.</summary>
        public const string InstallFolderName = "OpenFO4";

        private static readonly HashSet<string> ReservedDeviceNames = new(
            ["CON", "PRN", "AUX", "NUL", .. Enumerable.Range(1, 9).SelectMany(n => new[] { $"COM{n}", $"LPT{n}" })], StringComparer.OrdinalIgnoreCase);

        private static readonly char[] ForbiddenPathCharacters = [.. Path.GetInvalidPathChars(), '*', '?'];

        /// <summary>The user's input as a path: surrounding blanks and quotes (as pasted from a shell) dropped.</summary>
        public static string Normalize(string input) => input.Trim().Trim('"').Trim();

        /// <summary>The folder to suggest as the game folder: <paramref name="candidate"/> if it exists and holds the game executable, otherwise none.</summary>
        public static string? DefaultGameFolder(string candidate) =>
            Directory.Exists(candidate) && File.Exists(Path.Combine(candidate, GameExecutable)) ? candidate : null;

        /// <summary>
        /// The folder to suggest as the game folder: the one Steam's records name for Fallout 4, otherwise <paramref name="fallback"/>,
        /// otherwise none. What the search could not read is passed to <paramref name="note"/>.
        /// </summary>
        public static string? DefaultGameFolder(SteamGameFolderLocator steam, Action<string> note, string fallback = SteamGameFolder)
        {
            SteamGameFolderSearch search = steam.Find();
            foreach (string line in search.Notes) note(line);
            return search.Folder ?? DefaultGameFolder(fallback);
        }

        /// <summary>Why <paramref name="input"/> cannot be the game folder, or null when it can.</summary>
        public static string? GameFolderError(string input)
        {
            string path = Normalize(input);
            if (path.Length == 0) return "Choose the Fallout 4 folder.";
            if (!Path.IsPathFullyQualified(path) || !Directory.Exists(path)) return "This folder does not exist.";
            if (!File.Exists(Path.Combine(path, GameExecutable))) return $"{GameExecutable} was not found in this folder.";
            if (!File.Exists(Path.Combine(path, "Data", "Fallout4.esm"))) return @"Data\Fallout4.esm was not found in this folder.";
            return null;
        }

        /// <summary>
        /// Why <paramref name="input"/> cannot be the install folder, or null when it can. The folder must not exist yet, be empty, or
        /// hold an earlier OpenFO4 install: the install replaces and deletes files in it. <paramref name="gameFolderInput"/> is as the
        /// user typed it; the install folder may neither lie inside it nor contain it. Looks at the disk (the drive, the folder's contents).
        /// </summary>
        public static string? InstallFolderError(string input, string gameFolderInput)
        {
            string path = Normalize(input);
            if (path.Length == 0) return "Choose where to install.";
            if (path.IndexOfAny(ForbiddenPathCharacters) >= 0 || HasColonBeyondDrivePrefix(path)) return "The path contains characters that are not allowed.";
            if (!Path.IsPathFullyQualified(path)) return @"Enter a full path, such as D:\OpenFO4.";
            if (!TryResolve(path, out string full, out string? invalid)) return invalid;
            if (HasReservedName(full)) return $"{full} uses a name that Windows reserves, such as CON or NUL.";
            if (IsDriveLetterRoot(full) && !Directory.Exists(Path.GetPathRoot(full))) return $"The drive of {full} is not available.";
            if (IsRoot(full)) return $"{full} is a whole drive; choose a folder on it.";
            if (File.Exists(full)) return $"{full} is a file, not a folder.";

            string gameFolder = Normalize(gameFolderInput);
            if (gameFolder.Length > 0 && Path.IsPathFullyQualified(gameFolder) && TryResolve(gameFolder, out string game, out _))
            {
                if (IsSameOrInside(full, game)) return $"{full} is inside the Fallout 4 folder {game}; choose a folder outside it.";
                if (IsSameOrInside(game, full)) return $"{full} contains the Fallout 4 folder {game}; choose a folder that does not.";
            }
            return ContentError(full);
        }

        /// <summary>
        /// The folder the install goes into when the user chose <paramref name="chosenFolder"/>: that folder itself when it already holds an
        /// OpenFO4 install, otherwise <see cref="InstallFolderName"/> inside it. Input that is not a usable path is returned as it is, for
        /// the checks of <see cref="InstallFolderError"/> to reject.
        /// </summary>
        public static string InstallRoot(string chosenFolder)
        {
            string path = Normalize(chosenFolder);
            if (path.Length == 0 || path.IndexOfAny(ForbiddenPathCharacters) >= 0 || HasColonBeyondDrivePrefix(path) || !Path.IsPathFullyQualified(path)) return path;
            if (!TryResolve(path, out string full, out _)) return path;
            return new InstallLocations("", full).HoldsInstall() ? full : Path.Combine(full, InstallFolderName);
        }

        /// <summary>
        /// Why the install cannot go into <see cref="InstallRoot"/> of <paramref name="chosenFolder"/>, or null when it can: the chosen folder
        /// itself may hold anything, the install root below it must satisfy <see cref="InstallFolderError"/>.
        /// </summary>
        public static string? ChosenFolderError(string chosenFolder, string gameFolderInput)
        {
            string path = Normalize(chosenFolder);
            if (path.Length > 0 && !Path.IsPathFullyQualified(path) && path.IndexOfAny(ForbiddenPathCharacters) < 0) return @"Enter a full path, such as D:\Games.";
            if (path.Length > 0 && File.Exists(path)) return $"{path} is a file, not a folder.";
            return InstallFolderError(InstallRoot(chosenFolder), gameFolderInput);
        }

        public static string FullPath(string input) => Path.TrimEndingDirectorySeparator(Path.GetFullPath(Normalize(input)));

        private static string? ContentError(string full)
        {
            if (!Directory.Exists(full) || new InstallLocations("", full).HoldsInstall()) return null;
            try
            {
                return Directory.EnumerateFileSystemEntries(full).Any() ? $"{full} already contains other files; choose an empty folder or a previous OpenFO4 install." : null;
            }
            catch (Exception ex) when (ex is IOException or UnauthorizedAccessException)
            {
                return $"{full} cannot be read: {ex.Message}";
            }
        }

        private static bool TryResolve(string path, out string full, [NotNullWhen(false)] out string? invalid)
        {
            try
            {
                full = FullPath(path);
                invalid = null;
                if (!IsDevicePath(full)) return true;
                invalid = "This is not a folder path.";
            }
            catch (Exception ex) when (ex is ArgumentException or NotSupportedException or IOException)
            {
                full = "";
                invalid = $"This is not a valid folder path: {ex.Message}";
            }
            return false;
        }

        private static bool HasReservedName(string full) =>
            full[(Path.GetPathRoot(full)?.Length ?? 0)..].Split(Path.DirectorySeparatorChar, Path.AltDirectorySeparatorChar)
                .Any(segment => ReservedDeviceNames.Contains(segment.Split('.')[0].TrimEnd()));

        private static bool HasColonBeyondDrivePrefix(string path) =>
            path.Count(c => c == ':') > (path.Length >= 2 && char.IsAsciiLetter(path[0]) && path[1] == ':' ? 1 : 0);

        private static bool IsDevicePath(string full) => full.StartsWith(@"\\.\", StringComparison.Ordinal) || full.StartsWith(@"\\?\", StringComparison.Ordinal);

        private static bool IsDriveLetterRoot(string full) => Path.GetPathRoot(full) is { Length: >= 2 } root && root[1] == ':';

        private static bool IsRoot(string full) =>
            Path.TrimEndingDirectorySeparator(full).Equals(Path.TrimEndingDirectorySeparator(Path.GetPathRoot(full) ?? ""), StringComparison.OrdinalIgnoreCase);

        private static bool IsSameOrInside(string candidate, string container) =>
            candidate.Equals(container, StringComparison.OrdinalIgnoreCase)
            || candidate.StartsWith(container + Path.DirectorySeparatorChar, StringComparison.OrdinalIgnoreCase);
    }
}
