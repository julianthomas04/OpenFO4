using System.Diagnostics.CodeAnalysis;
using System.Text;
using System.Text.RegularExpressions;

namespace AssetConverter.Pipeline
{
    /// <summary>
    /// The content of the ownership marker: exactly <c>Asset Version &lt;version&gt;</c> (no newline; the version is the installer's version string, any token without whitespace), the version of the installer that converted
    /// the assets, which tells a game whether they need converting again. An install starts by emptying the marker (<see cref="WritePending"/>)
    /// since its assets are about to change, and writes the installer's version only once every step succeeded; a folder whose marker is
    /// empty (a failed or cancelled install) therefore never claims a complete asset set, and a development build's own <c>0.0.0-dev</c> is a
    /// real version, distinct from that. Markers from before the version existed hold a sentence instead and read as an unknown version.
    /// Every write goes to a temporary file that replaces the marker, so a reader sees the old content or the new, never a part.
    /// </summary>
    internal static partial class AssetVersionMarker
    {
        public const string Prefix = "Asset Version ";

        private const string TemporarySuffix = ".tmp";
        private static readonly UTF8Encoding Utf8WithoutBom = new(encoderShouldEmitUTF8Identifier: false);

        /// <exception cref="ArgumentException">The version does not match <see cref="InstallerVersion.Pattern"/>.</exception>
        public static string Format(string version) =>
            ValidVersion().IsMatch(version) ? $"{Prefix}{version}" : throw new ArgumentException($"'{version}' is not a valid version.", nameof(version));

        /// <summary>Marks the folder as holding no complete asset set of a known version.</summary>
        /// <exception cref="IOException">The marker could not be written.</exception>
        public static void WritePending(string path) => Replace(path, "");

        /// <exception cref="IOException">The marker could not be written.</exception>
        public static void Write(string path, string version) => Replace(path, Format(version));

        /// <returns>False when the file does not exist or holds anything but a version line (empty after an unfinished install, or the sentence older installs wrote).</returns>
        public static bool TryRead(string path, [NotNullWhen(true)] out string? version)
        {
            version = null;
            return File.Exists(path) && TryParse(File.ReadAllText(path, Utf8WithoutBom), out version);
        }

        public static bool TryParse(string content, [NotNullWhen(true)] out string? version)
        {
            var match = VersionLine().Match(content);
            version = match.Success ? match.Groups[1].Value : null;
            return match.Success;
        }

        private static void Replace(string path, string content)
        {
            string temporary = path + TemporarySuffix;
            try
            {
                File.WriteAllText(temporary, content, Utf8WithoutBom);
                File.Move(temporary, path, overwrite: true);
            }
            catch (Exception ex) when (ex is IOException or UnauthorizedAccessException)
            {
                DeleteTemporary(temporary);
                throw new IOException($"Cannot write the install marker '{path}': {ex.Message}", ex);
            }
        }

        private static void DeleteTemporary(string temporary)
        {
            try
            {
                File.Delete(temporary);
            }
            catch (Exception ex) when (ex is IOException or UnauthorizedAccessException)
            {
                Console.Error.WriteLine($"WARNING: could not remove the unfinished file '{temporary}': {ex.Message}");
            }
        }

        [GeneratedRegex("^" + InstallerVersion.Pattern + @"\z")]
        private static partial Regex ValidVersion();

        [GeneratedRegex("^Asset Version (" + InstallerVersion.Pattern + @")\r?\n?\z")]
        private static partial Regex VersionLine();
    }
}
