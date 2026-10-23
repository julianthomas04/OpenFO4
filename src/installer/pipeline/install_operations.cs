namespace AssetConverter.Pipeline
{
    /// <summary>Small install steps the full install and the dev subcommands both run.</summary>
    internal static class InstallOperations
    {
        private const int MaxReportedPurgeFailures = 10;

        /// <summary>
        /// Effect-shader shapes and particle systems need the shader archive's descriptor set; without it they would silently stay
        /// hidden legacy FX, so a missing or unreadable archive stops the conversion.
        /// </summary>
        public static void UseEffectShaderCatalog(string fo4Location)
        {
            NifConverter.EffectCatalog = FxpEffectShaderCatalog.FromGameInstall(fo4Location);
        }

        /// <summary>
        /// Copies the game's INI settings cascade into the install. Resolution order (Default &lt; Fallout4 &lt; Prefs &lt; Custom) is the
        /// engine's job. Fallout4.ini and Custom are optional (Custom only exists if the user made one); the engine refuses to start
        /// without Default and Prefs, so their absence fails the step with the message the setup view shows up front.
        /// </summary>
        /// <exception cref="InstallStartupException">A required INI is missing.</exception>
        public static void CopyGameInis(string fo4Location, string iniOut, string settingsDirectory)
        {
            if (GameIniFiles.MissingError(fo4Location, settingsDirectory) is { } missing) throw new InstallStartupException(missing);
            Directory.CreateDirectory(iniOut);
            var sources = GameIniFiles.Sources(fo4Location, settingsDirectory);
            int copied = 0;
            foreach (string src in GameIniFiles.Existing(sources))
            {
                File.Copy(src, Path.Combine(iniOut, Path.GetFileName(src)), overwrite: true);
                copied++;
            }
            Console.WriteLine($"Copied {copied} game INI(s) to {iniOut}");
        }

        /// <summary>
        /// Deletes every matching file under <paramref name="dir"/> on several threads (the deletes are file-system bound; a serial purge
        /// of the 160k GLBs takes about 40 s). A file a virus scanner, the search indexer or an Explorer preview briefly holds is retried
        /// with a growing delay; files that stay undeletable are collected and thrown as one <see cref="IOException"/> once the others are gone.
        /// </summary>
        /// <param name="wait">Pauses between delete attempts; defaults to sleeping, replaceable so tests need not.</param>
        public static void PurgeByExtension(string dir, string pattern, string description, Action<TimeSpan>? wait = null, CancellationToken ct = default)
        {
            if (!Directory.Exists(dir)) return;
            var files = Directory.GetFiles(dir, pattern, SearchOption.AllDirectories);
            if (files.Length == 0) return;
            Console.WriteLine($"Purging {files.Length:N0} existing {description} under {dir}...");

            wait ??= delay => ct.WaitHandle.WaitOne(delay);
            var failures = ParallelFailures.Collect(
                files.Length, IoParallelism.MaxThreads, index => DeleteWithRetry(files[index], wait, ct),
                stopAtFirstFailure: false, isFailure: ex => ex is IOException or UnauthorizedAccessException, ct: ct);

            if (failures.Count == 0) return;
            var reported = failures.OrderBy(failure => files[failure.Index], StringComparer.Ordinal).Take(MaxReportedPurgeFailures).ToList();
            throw new IOException(
                $"{failures.Count:N0} of {files.Length:N0} existing {description} under {dir} could not be deleted even after {IoRetry.DefaultDelays.Count} retries; " +
                $"another process (virus scanner, search indexer, Explorer preview, a running game) may be holding them. " +
                $"First {reported.Count}: {string.Join("; ", reported.Select(failure => $"{files[failure.Index]}: {failure.Error.Message}"))}",
                reported[0].Error);
        }

        private static void DeleteWithRetry(string file, Action<TimeSpan> wait, CancellationToken ct) =>
            IoRetry.Run(() => File.Delete(file), IoRetry.IsFileBusyOrDenied, IoRetry.DefaultDelays, wait, ct);
    }
}
