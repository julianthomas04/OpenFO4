using System.Text;
using System.Text.RegularExpressions;

namespace AssetConverter.Pipeline
{
    /// <summary>What an ownership marker records: the version of the installer that wrote it (informational) and the revision at which each asset group was converted completely.</summary>
    internal sealed record InstalledAssets(string Version, IReadOnlyDictionary<AssetGroup, int> Revisions)
    {
        public InstalledAssets Recording(AssetGroup group, int revision) =>
            this with { Revisions = new Dictionary<AssetGroup, int>(Revisions) { [group] = revision } };

        public InstalledAssets Keeping(IReadOnlySet<AssetGroup> groups) =>
            this with { Revisions = Revisions.Where(revision => groups.Contains(revision.Key)).ToDictionary(revision => revision.Key, revision => revision.Value) };
    }

    /// <summary>
    /// The content of the ownership marker: the line <c>Asset Version &lt;version&gt;</c> (the version of the installer that wrote the marker, any
    /// token without whitespace), then one <c>&lt;id&gt;=&lt;revision&gt;</c> line per asset group (see <see cref="AssetGroups"/>) whose converted
    /// output is complete, at the revision it was converted at; lines end with <c>\n</c>. Whether assets need converting again is decided by
    /// the revisions alone (see <see cref="ReconversionPlan"/>); the version is for logs and support.
    /// An empty marker (<see cref="WritePending"/>) says an install started without any group recorded. A marker that cannot be read, because a line is malformed, an id repeats or a revision
    /// is not a positive number, is as good as none, and so is a bare version line from before the groups existed (it records no group).
    /// Ids this installer does not know are ignored. Every write goes to a temporary file that replaces the marker, so a reader sees the old content or the new, never a part.
    /// </summary>
    internal static partial class AssetVersionMarker
    {
        public const string Prefix = "Asset Version ";

        private const string TemporarySuffix = ".tmp";
        private static readonly UTF8Encoding Utf8WithoutBom = new(encoderShouldEmitUTF8Identifier: false);
        private static readonly object UpdateLock = new();

        /// <exception cref="ArgumentException">The version does not match <see cref="InstallerVersion.Pattern"/>, or a revision is not positive.</exception>
        public static string Format(InstalledAssets assets)
        {
            if (!ValidVersion().IsMatch(assets.Version))
                throw new ArgumentException($"'{assets.Version}' is not a valid version.", nameof(assets));
            if (assets.Revisions.Any(revision => revision.Value < 1))
                throw new ArgumentException("A revision must be a positive number.", nameof(assets));

            var text = new StringBuilder().Append(Prefix).Append(assets.Version).Append('\n');
            foreach (var group in AssetGroups.All.Where(assets.Revisions.ContainsKey))
                text.Append(AssetGroups.Id(group)).Append('=').Append(assets.Revisions[group]).Append('\n');
            return text.ToString();
        }

        /// <summary>Marks the folder as holding no complete asset group.</summary>
        /// <exception cref="IOException">The marker could not be written.</exception>
        public static void WritePending(string path) => Replace(path, "");

        /// <exception cref="IOException">The marker could not be written.</exception>
        public static void Write(string path, InstalledAssets assets) => Replace(path, Format(assets));

        /// <returns>Null when the file does not exist or holds no readable record (empty after a started install, or anything unreadable).</returns>
        public static InstalledAssets? Read(string path) => File.Exists(path) ? Parse(File.ReadAllText(path, Utf8WithoutBom)) : null;

        public static InstalledAssets? Parse(string content)
        {
            var lines = content.Split('\n').Select(line => line.TrimEnd('\r')).ToList();
            if (lines[^1].Length == 0)
                lines.RemoveAt(lines.Count - 1);

            var header = lines.Count == 0 ? null : VersionLine().Match(lines[0]);
            if (header is not { Success: true })
                return null;

            var seen = new HashSet<string>();
            var revisions = new Dictionary<AssetGroup, int>();
            foreach (string line in lines.Skip(1))
            {
                var match = GroupLine().Match(line);
                if (!match.Success || !seen.Add(match.Groups[1].Value) || !int.TryParse(match.Groups[2].Value, out int revision) || revision < 1)
                    return null;
                if (AssetGroups.TryFromId(match.Groups[1].Value, out var group))
                    revisions[group] = revision;
            }
            return new InstalledAssets(header.Groups[1].Value, revisions);
        }

        /// <summary>
        /// Rewrites the record as <paramref name="change"/> returns it, given the current one (null when there is none or it is unreadable), or empties the marker when it returns null.
        /// Writers in this process take turns, so none loses another's change.
        /// </summary>
        /// <exception cref="IOException">The marker could not be written.</exception>
        public static void Update(string path, Func<InstalledAssets?, InstalledAssets?> change)
        {
            lock (UpdateLock)
            {
                var updated = change(Read(path));
                if (updated is null)
                    WritePending(path);
                else
                    Write(path, updated);
            }
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

        [GeneratedRegex("^Asset Version (" + InstallerVersion.Pattern + @")\z")]
        private static partial Regex VersionLine();

        [GeneratedRegex(@"^([^=\s]+)=([0-9]+)\z")]
        private static partial Regex GroupLine();
    }
}
