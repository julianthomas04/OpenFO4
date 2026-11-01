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
            if (!packaged) return DeclaringGameFiles(resolve(null));
            InstallerResources? resources = null;
            await InstallProgress.RunLane(InstallSteps.UnpackLane, () => Task.Run(() =>
            {
                int phase = InstallProgress.BeginPhase(InstallSteps.UnpackPayload, null);
                resources = DeclaringGameFiles(resolve(new UnpackProgress(
                    count => AnnounceFiles(phase, count), DeclareGameFiles, () => InstallProgress.Tick(phase))));
                InstallProgress.EndPhase(phase);
            })).ConfigureAwait(false);
            return resources!;
        }

        /// <summary>For a payload that was unpacked by an earlier run, which reports no count while unpacking.</summary>
        private static InstallerResources DeclaringGameFiles(InstallerResources resources)
        {
            DeclareGameFiles(resources.Game.Files.Count);
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
