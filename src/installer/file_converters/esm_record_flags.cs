namespace AssetConverter
{
    internal static class RecordFlags
    {
        /// <summary>The record is deleted and never spawns in-game.</summary>
        public const int Deleted = 0x20;

        /// <summary>The placed reference starts disabled.</summary>
        public const int InitiallyDisabled = 0x800;
    }
}
