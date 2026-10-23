namespace AssetConverter.Audio
{
    /// <summary>The encoding of a source audio entry, which tells ffmpeg how to read it.</summary>
    internal enum AudioContainer
    {
        Xwma,
        Wav,
    }

    /// <summary>
    /// Which game files are audio and where their conversions go: the output key of an entry is its archive path lowercased, with
    /// forward slashes and the extension replaced by <c>.flac</c> (<c>Music\Special\MUS_MainTheme.xwm</c> becomes
    /// <c>music/special/mus_maintheme.flac</c>), below <see cref="InstallLayout.AudioFolder"/>. The engine derives the same key.
    /// </summary>
    internal static class AudioLayout
    {
        /// <summary>The base game's archives that hold sound and music, in the order they are applied: an entry of a later archive replaces the same path of an earlier one.</summary>
        public static readonly IReadOnlyList<string> Archives = ["Fallout4 - Sounds.ba2", "Fallout4 - Startup.ba2"];

        public const string FlacExtension = ".flac";
        public const string XwmaExtension = ".xwm";
        public const string WavExtension = ".wav";

        /// <summary>Voice lines: out of scope, wherever they sit.</summary>
        private const string VoiceExtension = ".fuz";

        private const string GameFolderPrefix = "Data\\";

        private static readonly string[] Roots = ["Sound\\", "Music\\"];

        /// <summary>Whether <paramref name="archivePath"/> is a sound or music entry to convert: below <c>Sound\</c> or <c>Music\</c> and not a voice file.</summary>
        public static bool IsInScope(string archivePath)
        {
            string normalized = ArchivePath.Normalize(archivePath);
            return Roots.Any(root => normalized.StartsWith(root, StringComparison.OrdinalIgnoreCase))
                && !normalized.EndsWith(VoiceExtension, StringComparison.OrdinalIgnoreCase);
        }

        /// <exception cref="AudioConversionException">The extension is neither <c>.xwm</c> nor <c>.wav</c>.</exception>
        public static AudioContainer ContainerOf(string archivePath)
        {
            string extension = Path.GetExtension(archivePath);
            if (extension.Equals(XwmaExtension, StringComparison.OrdinalIgnoreCase)) return AudioContainer.Xwma;
            if (extension.Equals(WavExtension, StringComparison.OrdinalIgnoreCase)) return AudioContainer.Wav;
            throw new AudioConversionException($"Audio entry '{archivePath}' is neither {XwmaExtension} nor {WavExtension}; the installer does not know how to convert it.");
        }

        /// <summary>
        /// The output key of an audio file. Records that name a file (a music track's ANAM) spell it from the game folder, as
        /// <c>data\Music\...</c> or <c>\Data\Music\...</c>; that prefix is dropped so the key equals the one of the archive entry.
        /// </summary>
        /// <exception cref="AudioConversionException">The entry is not a convertible audio file, or its path leaves the audio folder.</exception>
        public static string ToAudioKey(string archivePath)
        {
            ContainerOf(archivePath);
            string normalized = StripGameFolderPrefix(ArchivePath.Normalize(archivePath));
            if (normalized.Split('\\').Any(segment => segment is "" or "." or ".."))
                throw new AudioConversionException($"Audio entry '{archivePath}' has a path that cannot be placed below the audio folder.");
            if (!Roots.Any(root => normalized.StartsWith(root, StringComparison.OrdinalIgnoreCase)))
                throw new AudioConversionException($"Audio entry '{archivePath}' is not below Sound\\ or Music\\, so it is outside the audio scope.");
            string withoutExtension = normalized[..^Path.GetExtension(normalized).Length];
            return (withoutExtension + FlacExtension).Replace('\\', '/').ToLowerInvariant();
        }

        private static string StripGameFolderPrefix(string normalized)
        {
            string withoutRoot = normalized.TrimStart('\\');
            return withoutRoot.StartsWith(GameFolderPrefix, StringComparison.OrdinalIgnoreCase) ? withoutRoot[GameFolderPrefix.Length..] : normalized;
        }

        public static string OutputPath(string audioRoot, string key) => Path.Combine(audioRoot, key.Replace('/', Path.DirectorySeparatorChar));

        /// <summary>The file that records the conversion stamp of one source archive, beside the audio it produced.</summary>
        public static string MarkerPath(string audioRoot, string archiveName) => Path.Combine(audioRoot, $".{archiveName}{ConversionStamp.SidecarSuffix}");
    }
}
