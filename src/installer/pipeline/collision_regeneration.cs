namespace AssetConverter.Pipeline
{
    /// <summary>
    /// Converts the collision group again against an existing install, leaving the install as a normal install leaves it: the group's output is purged the way
    /// <see cref="GroupPurge"/> purges it for a reconversion, and the marker vouches for the group at its current revision afterwards, so a later install keeps it.
    /// </summary>
    internal static class CollisionRegeneration
    {
        /// <exception cref="IOException">The marker could not be written.</exception>
        public static void Run(InstallLocations paths, Action extract)
        {
            GroupPurge.Purge(AssetGroup.Collision, paths);
            extract();
            paths.RecordGroupAtCurrentRevision(AssetGroup.Collision);
        }
    }
}
