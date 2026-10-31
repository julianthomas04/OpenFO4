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

        /// <summary>
        /// Records that every asset of the install was converted by the installer of <paramref name="assetVersion"/>; the install fails when
        /// this cannot be written, since the marker is what lets a game use the assets.
        /// </summary>
        /// <exception cref="IOException">The marker could not be written.</exception>
        public void MarkAssetsComplete(string assetVersion) => AssetVersionMarker.Write(OwnershipMarker, assetVersion);

        /// <summary>
        /// Whether the install root is, or has been, an OpenFO4 install: it carries the ownership marker, or (installs from before the
        /// marker existed) both the install log and the database.
        /// </summary>
        public bool HoldsInstall() => File.Exists(OwnershipMarker) || (File.Exists(LogFile) && File.Exists(DatabasePath));

        /// <summary>Creates the install root, claims it with the ownership marker, which stops vouching for any earlier asset version, and creates the output folders.</summary>
        public void CreateOutputDirectories()
        {
            Directory.CreateDirectory(InstallRoot);
            AssetVersionMarker.WritePending(OwnershipMarker);
            foreach (string directory in new[] { StagingRoot, MeshesOut, TexturesOut, DataOut })
                Directory.CreateDirectory(directory);
        }
    }
}
