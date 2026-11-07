using System.Security.Cryptography;
using System.Text;
using AssetConverter.UiLayout;
using AssetConverter.Video;

namespace AssetConverter.Pipeline
{
    /// <summary>
    /// What the converted assets are made from besides the installer itself: the language the game is installed in (the localized files, the string
    /// tables and the fonts depend on it) and a fingerprint of the game files the installer reads. An install whose record names other inputs has
    /// nothing worth keeping, since a group's revision only vouches for the installer's side of its output.
    /// </summary>
    /// <param name="Language">The installed language code (<c>sLanguage</c>, lowercase letters).</param>
    /// <param name="Files">The <see cref="GameFingerprint"/> of the game files.</param>
    internal sealed record GameInputs(string Language, string Files)
    {
        /// <exception cref="ConversionStepException">The game's INIs do not state a usable language.</exception>
        /// <exception cref="IOException">The language or the game files could not be read.</exception>
        public static GameInputs Read(InstallLocations paths) =>
            new(GameLanguage.Resolve(paths.Fo4Location, paths.SettingsDirectory), GameFingerprint.Scan(paths.GameDataDirectory));
    }

    /// <summary>A game file as the fingerprint sees it: its path below the Data folder, its size and its last write time.</summary>
    internal readonly record struct GameFileStamp(string RelativePath, long Length, DateTime LastWriteUtc);

    /// <summary>
    /// A cheap, deterministic stand-in for "the game files the installer reads are the ones the assets were converted from": a SHA-256 over the
    /// path, size and last write time of the game's master (<c>Fallout4.esm</c>), its archives (<c>Fallout4 - *.ba2</c>) and its videos
    /// (<c>Data\Video\*.bk2</c>). No file content is read. A game update or a repair that rewrites one of these files changes it; downloadable
    /// content and mods, which the installer does not read, do not.
    /// </summary>
    internal static class GameFingerprint
    {
        private const string MasterFile = "Fallout4.esm";
        private const string ArchivePattern = "Fallout4 - *.ba2";

        /// <returns>64 lowercase hex digits; the same for the same files whatever the order they are given in.</returns>
        public static string Compute(IEnumerable<GameFileStamp> files)
        {
            var lines = files
                .Select(file => $"{file.RelativePath.Replace('\\', '/').ToLowerInvariant()}|{file.Length}|{file.LastWriteUtc.Ticks}")
                .Order(StringComparer.Ordinal);
            return Convert.ToHexString(SHA256.HashData(Encoding.UTF8.GetBytes(string.Join('\n', lines)))).ToLowerInvariant();
        }

        /// <exception cref="IOException">The game's Data folder could not be read.</exception>
        public static string Scan(string gameDataDirectory) => Compute(Stamps(gameDataDirectory));

        private static IEnumerable<GameFileStamp> Stamps(string dataDirectory)
        {
            var files = new List<string>();
            if (File.Exists(Path.Combine(dataDirectory, MasterFile)))
                files.Add(Path.Combine(dataDirectory, MasterFile));
            if (Directory.Exists(dataDirectory))
                files.AddRange(Directory.EnumerateFiles(dataDirectory, ArchivePattern, SearchOption.TopDirectoryOnly));
            string videos = VideoLayout.SourceDirectory(dataDirectory);
            if (Directory.Exists(videos))
                files.AddRange(Directory.EnumerateFiles(videos, "*" + VideoLayout.SourceExtension, SearchOption.TopDirectoryOnly));

            return files.Select(path =>
            {
                var info = new FileInfo(path);
                return new GameFileStamp(Path.GetRelativePath(dataDirectory, path), info.Length, info.LastWriteTimeUtc);
            });
        }
    }
}
