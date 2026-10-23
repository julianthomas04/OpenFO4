using AssetConverter.Progress;

namespace AssetConverter.Pipeline
{
    /// <summary>
    /// The first work of a packaged install: unpacking the payload the installer carries, a part of the Initializing step counting the
    /// files written. An installer run from a repository has nothing to unpack and declares no such part.
    /// </summary>
    internal static class PayloadUnpackStep
    {
        /// <param name="packaged">Whether the installer carries a payload.</param>
        /// <param name="resolve">Finds the installer's resources, unpacking the payload when it has one and reporting that to the progress it is given.</param>
        /// <exception cref="InstallStartupException">The payload could not be unpacked; the step is marked failed.</exception>
        public static async Task<InstallerResources> RunAsync(bool packaged, Func<UnpackProgress?, InstallerResources> resolve)
        {
            if (!packaged) return resolve(null);
            InstallerResources? resources = null;
            await InstallProgress.RunLane(InstallSteps.UnpackLane, () => Task.Run(() =>
            {
                int phase = InstallProgress.BeginPhase(InstallSteps.UnpackPayload, 0);
                resources = resolve(new UnpackProgress(count => InstallProgress.AddTotal(phase, count), () => InstallProgress.Tick(phase)));
                InstallProgress.EndPhase(phase);
            })).ConfigureAwait(false);
            return resources!;
        }
    }
}
