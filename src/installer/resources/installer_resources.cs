namespace AssetConverter
{
    /// <summary>
    /// Where the installer finds what it runs on: its data files, the tools it starts and the game it installs. In a repository checkout
    /// they are the repo's own files and the tools on the system; in a packaged installer they are the extracted payload, which
    /// holds every one of them. Everything under <see cref="Root"/> has the same repo-relative layout in both.
    /// </summary>
    internal sealed class InstallerResources
    {
        public const string RuntimeFolder = "runtime";
        public const string PythonFolder = "python";
        public const string JavaFolder = "jre";
        public const string GameFolder = "game";
        private const string JavaExecutable = "java.exe";
        private const string FfmpegExecutable = "ffmpeg.exe";
        private const string FfmpegCheckoutFolder = "build/librempeg";
        private const string FfmpegPayloadFolder = "third_party/librempeg";
        private const string Bink2DecoderExecutable = "Bink2Decoder.exe";
        private const string Bink2DecoderCheckoutFolder = "tools/bink2-decoder/bin/Release/net8.0";
        private const string Bink2DecoderPayloadFolder = "third_party/bink2-decoder";

        private static readonly string[] EngineBuildProfiles = ["release", "debug"];
        private static readonly string[] SystemPythonNames = ["python.exe", "python3.exe", "py.exe"];
        private static readonly string[] FfdecJavaOptions =
            ["-Xmx1024m", "-Djava.net.preferIPv4Stack=true", "-Djna.nosys=true", "-Djava.util.Arrays.useLegacyMergeSort=true"];

        private readonly string? _bundledRuntimeRoot;
        private readonly Func<string?> _systemJava;

        private InstallerResources(string root, string? bundledRuntimeRoot, GameFiles game, Func<string?> systemJava)
        {
            Root = root;
            _bundledRuntimeRoot = bundledRuntimeRoot;
            Game = game;
            _systemJava = systemJava;
        }

        /// <summary>The folder the repo-relative resource paths start at.</summary>
        public string Root { get; }

        public bool IsPackaged => _bundledRuntimeRoot != null;

        public GameFiles Game { get; }

        /// <summary>The resources of a repository checkout: tools from the system, the game from the engine's build output when it has been built.</summary>
        public static InstallerResources ForRepository(string repoRoot, Func<string?>? systemJava = null)
        {
            string target = Path.Combine(repoRoot, "src", "engine", "target");
            string? builtGame = EngineBuildProfiles.Select(profile => Path.Combine(target, profile, GameFiles.ExecutableName)).FirstOrDefault(File.Exists);
            return new InstallerResources(repoRoot, null,
                builtGame == null ? GameFiles.None : new GameFiles([builtGame], Required: false), systemJava ?? FindSystemJava);
        }

        /// <summary>The resources of an extracted payload: its own tools and its own game files (the whole game folder, subfolders included), which an install requires.</summary>
        public static InstallerResources ForPayload(string payloadRoot)
        {
            string gameFolder = Path.Combine(payloadRoot, GameFolder);
            string[] gameFiles = Directory.Exists(gameFolder) ? [.. Directory.EnumerateFiles(gameFolder, "*", SearchOption.AllDirectories).Order(StringComparer.OrdinalIgnoreCase)] : [];
            return new InstallerResources(payloadRoot, Path.Combine(payloadRoot, RuntimeFolder),
                new GameFiles(gameFiles, Required: true, Root: gameFolder), () => null);
        }

        /// <summary>Absolute path of a resource given relative to <see cref="Root"/>; throws if it does not exist.</summary>
        public string Require(string relativePath)
        {
            string full = Path.GetFullPath(Path.Combine(Root, relativePath));
            if (!File.Exists(full))
                throw new FileNotFoundException($"Required installer resource '{relativePath}' not found at '{full}'.", full);
            return full;
        }

        /// <summary>The Python the collision worker runs on: the bundled one, or the first on PATH; null when there is none.</summary>
        public string? PythonExecutable() =>
            _bundledRuntimeRoot != null
                ? Path.Combine(_bundledRuntimeRoot, PythonFolder, "python.exe")
                : SystemPythonNames.Select(name => ExecutableSearch.FindOnPath(name, Environment.GetEnvironmentVariable("PATH"))).FirstOrDefault(found => found != null);

        /// <summary>The Java of the system: the one in <c>JAVA_HOME</c>, else the first on PATH; null when there is none.</summary>
        public static string? FindSystemJava() =>
            FindSystemJava(Environment.GetEnvironmentVariable("JAVA_HOME"), Environment.GetEnvironmentVariable("PATH"));

        internal static string? FindSystemJava(string? javaHome, string? pathVariable)
        {
            if (!string.IsNullOrWhiteSpace(javaHome))
            {
                string candidate = Path.Combine(javaHome.Trim().Trim('"'), "bin", JavaExecutable);
                if (File.Exists(candidate)) return candidate;
            }
            return ExecutableSearch.FindOnPath(JavaExecutable, pathVariable);
        }

        internal static string MissingJavaMessage(string? javaHome)
        {
            const string Remedy = $"set JAVA_HOME to a Java installation or put {JavaExecutable} on PATH.";
            if (string.IsNullOrWhiteSpace(javaHome)) return $"No Java found for ffdec: {Remedy}";
            string expected = Path.Combine(javaHome.Trim().Trim('"'), "bin", JavaExecutable);
            return $"No Java found for ffdec: JAVA_HOME is set but contains no bin{Path.DirectorySeparatorChar}{JavaExecutable} (expected '{expected}') and PATH has none either; {Remedy}";
        }

        /// <summary>How to start JPEXS ffdec: its jar on the bundled Java, or on the system's Java in a checkout, with the same options either way.</summary>
        /// <exception cref="FfdecException">A checkout has no Java on the system.</exception>
        public ToolCommand Ffdec()
        {
            string jar = Path.Combine(Root, IsPackaged ? "third_party" : "build", "ffdec", "ffdec-cli.jar");
            string[] arguments = [.. FfdecJavaOptions, "-jar", jar];

            if (_bundledRuntimeRoot == null)
            {
                string java = _systemJava() ?? throw new FfdecException(MissingJavaMessage(Environment.GetEnvironmentVariable("JAVA_HOME")));
                return new ToolCommand(java, arguments, new Dictionary<string, string>(), [java, jar]);
            }

            string javaHome = Path.Combine(_bundledRuntimeRoot, JavaFolder);
            string bundledJava = Path.Combine(javaHome, "bin", JavaExecutable);
            return new ToolCommand(bundledJava, arguments, new Dictionary<string, string> { ["JAVA_HOME"] = javaHome }, [bundledJava, jar]);
        }

        /// <summary>The Librempeg ffmpeg that converts the game's audio: the one built into <c>build/librempeg</c> in a checkout, the bundled one in a payload.</summary>
        public ToolCommand Ffmpeg() =>
            ToolCommand.Program(Path.GetFullPath(Path.Combine(Root, IsPackaged ? FfmpegPayloadFolder : FfmpegCheckoutFolder, FfmpegExecutable)));

        /// <summary>The standalone Bink2 decoder that converts the game's videos: the Release build of <c>tools/bink2-decoder</c> in a checkout, the bundled self-contained one in a payload.</summary>
        public ToolCommand Bink2Decoder() =>
            ToolCommand.Program(Path.GetFullPath(Path.Combine(Root, IsPackaged ? Bink2DecoderPayloadFolder : Bink2DecoderCheckoutFolder, Bink2DecoderExecutable)));
    }
}
