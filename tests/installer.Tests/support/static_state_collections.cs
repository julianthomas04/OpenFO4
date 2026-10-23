namespace InstallerTests
{
    /// <summary>Names of xunit collections whose members must not run concurrently because they share process-wide static state.</summary>
    internal static class StaticStateCollections
    {
        /// <summary>
        /// The NIF converter's process-wide state: its effect catalogue, texture index and conversion statistics (every batched conversion
        /// resets them), plus the material census the palette baker and the MODC synthesis read. Tests that set that state, run a batched
        /// conversion or assert on the statistics belong here.
        /// </summary>
        public const string NifConverterState = "NIF converter process-wide state";
    }
}
