using AssetConverter.Progress;

namespace AssetConverter.Pipeline
{
    /// <summary>
    /// The first work of a packaged install: unpacking the payload the installer carries, a part of the Initializing step counting the
    /// files written. An installer run from a repository has nothing to unpack and declares no such part. Either way, the number of game
    /// files to install is declared as soon as it is known, so the Initializing step has its final total before anything is copied.
    /// </summary>
    internal static class PayloadUnpackStep
    {
        /// <param name="packaged">Whether the installer carries a payload.</param>
        /// <param name="resolve">Finds the installer's resources, unpacking the payload when it has one and reporting that to the progress it is given.</param>
        /// <exception cref="InstallStartupException">The payload could not be unpacked; the step is marked failed.</exception>
        public static async Task<InstallerResources> RunAsync(bool packaged, Func<UnpackProgress?, InstallerResources> resolve)
        {
            if (!packaged) return DeclaringGameFiles(resolve(null), listed: null);
            InstallerResources? resources = null;
            await InstallProgress.RunLane(InstallSteps.UnpackLane, () => Task.Run(() =>
            {
                int phase = InstallProgress.BeginPhase(InstallSteps.UnpackPayload, null);
                long? listed = null;
                resources = DeclaringGameFiles(resolve(new UnpackProgress(
                    count => AnnounceFiles(phase, count), count => { listed = count; DeclareGameFiles(count); }, () => InstallProgress.Tick(phase))), listed);
                InstallProgress.EndPhase(phase);
            })).ConfigureAwait(false);
            return resources!;
        }

        /// <summary>Declares the game files found, which for a payload unpacked by an earlier run is the first time they are counted.</summary>
        /// <param name="listed">The count the payload's listing gave while unpacking; null when it gave none.</param>
        /// <exception cref="InstallStartupException">The listing and the unpacked files disagree on the number of game files.</exception>
        private static InstallerResources DeclaringGameFiles(InstallerResources resources, long? listed)
        {
            int found = resources.Game.Files.Count;
            if (listed is { } count && count != found)
                throw new InstallStartupException(
                    $"The installer's payload lists {count} game files, but {found} were unpacked into '{resources.Root}'; the installer is damaged or its payload layout changed.");
            DeclareGameFiles(found);
            return resources;
        }

        private static void DeclareGameFiles(long count) => InstallProgress.Declare(InstallSteps.InstallGame, count);

        private static void AnnounceFiles(int phase, long count)
        {
            InstallProgress.AddTotal(phase, count);
            InstallProgress.SealTotal(phase);
        }
    }
}
