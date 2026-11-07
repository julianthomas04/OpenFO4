using AssetConverter.Video;

namespace AssetConverter.Pipeline
{
    /// <summary>
    /// Every path of one install: the game to read from, the folder holding the INIs the game's first launch wrote (null for the
    /// default one) and the install root to write to, laid out as <see cref="InstallLayout"/> describes.
    /// </summary>
    internal sealed record InstallLocations(string Fo4Location, string InstallRoot, string? SettingsFolder = null)
    {
        public string SettingsDirectory => SettingsFolder ?? GameIniFiles.DocumentsDirectory();

        public string StagingRoot => Path.Combine(InstallRoot, InstallLayout.StagingFolder);

        public string AssetsRoot => Path.Combine(InstallRoot, InstallLayout.AssetsFolder);

        public string MeshesOut => Path.Combine(AssetsRoot, InstallLayout.MeshesFolder);

        public string TexturesOut => Path.Combine(AssetsRoot, InstallLayout.TexturesFolder);

        public string AudioOut => Path.Combine(AssetsRoot, InstallLayout.AudioFolder);

        public string VideoOut => Path.Combine(AssetsRoot, InstallLayout.VideoFolder);

        /// <summary>The database and the game INI cascade.</summary>
        public string DataOut => Path.Combine(AssetsRoot, InstallLayout.DataFolder);

        public string FontsOut => InstallLayout.FontsDirectory(AssetsRoot);


        public string PipboyArtOut => InstallLayout.PipboyDirectory(AssetsRoot);

        public string UiLayoutOut => InstallLayout.UiLayoutDirectory(AssetsRoot);

        public string DatabasePath => Path.Combine(DataOut, InstallLayout.DatabaseFileName);

        public string GameDataDirectory => Path.Combine(Fo4Location, "Data");

        public string GameVideoDirectory => VideoLayout.SourceDirectory(GameDataDirectory);

        public string GameEsm => Path.Combine(GameDataDirectory, "Fallout4.esm");

        public string StagedEsm => Path.Combine(StagingRoot, "Fallout4.esm");

        public string StagedMeshes => Path.Combine(StagingRoot, "Meshes");

        public string StagedMaterials => Path.Combine(StagingRoot, "Materials");

        public string GameExecutable => Path.Combine(InstallRoot, GameFiles.ExecutableName);

        public string LogFile => Path.Combine(InstallRoot, InstallLayout.InstallLogFileName);

        public string OwnershipMarker => Path.Combine(InstallRoot, InstallLayout.OwnershipMarkerFileName);

        /// <summary>Rewrites the marker to vouch only for the groups <paramref name="plan"/> keeps, as they were installed, since the others are about to change; the marker names <paramref name="inputs"/> from now on.</summary>
        /// <exception cref="IOException">The marker could not be written.</exception>
        public void WithdrawReconverted(ReconversionPlan plan, string installerVersion, GameInputs? inputs) =>
            AssetVersionMarker.Update(OwnershipMarker, installed =>
            {
                var kept = installed?.Keeping(plan.Kept);
                return kept is { Revisions.Count: > 0 } ? kept with { Version = installerVersion, Inputs = inputs } : null;
            });

        /// <summary>Adds <paramref name="group"/> at its current revision to the marker, since its converted output is complete.</summary>
        /// <exception cref="IOException">The marker could not be written.</exception>
        public void MarkGroupComplete(AssetGroup group, string installerVersion, GameInputs? inputs) =>
            AssetVersionMarker.Update(OwnershipMarker, installed =>
                (installed ?? new InstalledAssets(installerVersion, new Dictionary<AssetGroup, int>())).Recording(group, AssetGroups.CurrentRevisions[group]) with { Version = installerVersion, Inputs = inputs });

        /// <summary>
        /// Whether the install root is, or has been, an OpenFO4 install: it carries the ownership marker, or (installs from before the
        /// marker existed) both the install log and the database.
        /// </summary>
        public bool HoldsInstall() => File.Exists(OwnershipMarker) || (File.Exists(LogFile) && File.Exists(DatabasePath));

        /// <summary>Creates the install root, claims it with an empty ownership marker unless it has one (an earlier install's record stays until the pipeline withdraws what it converts again), and creates the output folders.</summary>
        public void CreateOutputDirectories()
        {
            Directory.CreateDirectory(InstallRoot);
            if (!File.Exists(OwnershipMarker))
                AssetVersionMarker.WritePending(OwnershipMarker);
            foreach (string directory in new[] { StagingRoot, MeshesOut, TexturesOut, DataOut })
                Directory.CreateDirectory(directory);
        }
    }
}
