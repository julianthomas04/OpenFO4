namespace AssetConverter.Pipeline
{
    /// <summary>Puts the game's files into the install folder, with the folders they sit in. A file is copied beside its destination first and then moved over it, so a destination is either the old file or the complete new one.</summary>
    internal static class GameFileInstaller
    {
        private const string PartialSuffix = ".installing";

        /// <exception cref="IOException">A file could not be written; the message names it and the folder.</exception>
        public static void Install(GameFiles game, string installRoot, Action<string> installed, CancellationToken ct)
        {
            var names = game.Files.Select(game.InstalledPath).ToList();
            foreach (string name in names)
                RequireInsideInstallRoot(name);
            foreach (string source in game.Files)
            {
                ct.ThrowIfCancellationRequested();
                string name = game.InstalledPath(source);
                CopyAtomically(source, Path.Combine(installRoot, name), installRoot);
                installed(name);
            }
        }

        private static void RequireInsideInstallRoot(string relativePath)
        {
            if (Path.IsPathRooted(relativePath) || relativePath.Split(Path.DirectorySeparatorChar, Path.AltDirectorySeparatorChar).Contains(".."))
                throw new InvalidOperationException($"The game file '{relativePath}' would be installed outside the install folder.");
        }

        private static void CopyAtomically(string source, string destination, string installRoot)
        {
            string partial = destination + PartialSuffix;
            try
            {
                Directory.CreateDirectory(Path.GetDirectoryName(destination)!);
                File.Copy(source, partial, overwrite: true);
                File.Move(partial, destination, overwrite: true);
            }
            catch (Exception ex) when (ex is IOException or UnauthorizedAccessException)
            {
                TryDelete(partial);
                throw new IOException($"Cannot install {Path.GetRelativePath(installRoot, destination)} into '{installRoot}' (is the game running?): {ex.Message}", ex);
            }
        }

        private static void TryDelete(string file)
        {
            try
            {
                File.Delete(file);
            }
            catch (Exception ex) when (ex is IOException or UnauthorizedAccessException)
            {
                Console.Error.WriteLine($"WARNING: could not remove the unfinished file '{file}': {ex.Message}");
            }
        }
    }
}
