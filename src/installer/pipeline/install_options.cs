namespace AssetConverter.Pipeline
{
    /// <summary>What the user chose about one full install besides the folders, and what the install brings along.</summary>
    /// <param name="KeepStaging">Leave the installer's temporary files in place after a successful install, so the next one starts warm.</param>
    /// <param name="AssetVersion">The version written to the ownership marker once the install succeeded; the installer's own by default.</param>
    /// <param name="Game">The game files the install puts into the install folder; none when the installer has no game to install.</param>
    /// <param name="UnpacksPayload">The installer carries a payload that its first step unpacks; the step is part of the planned steps.</param>
    internal sealed record InstallOptions(bool KeepStaging = false, GameFiles? Game = null, string? AssetVersion = null, bool UnpacksPayload = false)
    {
        public string EffectiveAssetVersion => AssetVersion ?? InstallerVersion.Current;
    }
}
