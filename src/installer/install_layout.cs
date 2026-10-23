namespace AssetConverter
{
    /// <summary>
    /// Folder and file names of the install root and of the asset root the engine reads, the single source of truth on the installer side.
    /// The engine's <c>assets/layout.rs</c> carries the same names, except for the installer-private staging folder; a test there compares them.
    /// Everything is lowercase. Folders below these (<c>Actors</c>, <c>Terrain</c>, ...) keep the casing of the game's own archive paths.
    /// </summary>
    public static class InstallLayout
    {
        /// <summary>Installer-private intermediates below the install root.</summary>
        public const string StagingFolder = "staging";

        /// <summary>The final artifacts the engine consumes, below the install root.</summary>
        public const string AssetsFolder = "assets";

        public const string MeshesFolder = "meshes";
        public const string TexturesFolder = "textures";
        public const string InterfaceFolder = "interface";
        public const string FontsFolder = "fonts";
        public const string PipboyFolder = "pipboy";

        /// <summary>The FLAC files converted from the game's sound and music, below the path of each source with its extension replaced.</summary>
        public const string AudioFolder = "audio";

        /// <summary>The MP4 files converted from the game's Bink2 videos, one per source named by its lowercased base name.</summary>
        public const string VideoFolder = "video";

        /// <summary>The per-movie UI layout documents and the installed language's translate table and font configuration.</summary>
        public const string LayoutFolder = "layout";

        /// <summary>The database and the game INI cascade.</summary>
        public const string DataFolder = "data";

        /// <summary>The WGSL translated from the game's shaders.</summary>
        public const string ShadersFolder = "shaders";

        /// <summary>The file by which the installer claims a folder as an OpenFO4 install, below the install root; it holds the asset version, see <see cref="Pipeline.AssetVersionMarker"/>.</summary>
        public const string OwnershipMarkerFileName = ".openfo4-install";

        /// <summary>The installer's log, below the install root.</summary>
        public const string InstallLogFileName = "install.log";

        /// <summary>The database, below <see cref="DataFolder"/>.</summary>
        public const string DatabaseFileName = "data.db";

        /// <summary>The Pip-Boy art manifest, below <see cref="PipboyFolder"/>; the frame files it names sit below the same folder.</summary>
        public const string PipboyArtManifestFileName = "art_manifest.json";

        /// <summary>The installed language's UI string table, below <see cref="LayoutFolder"/>.</summary>
        public const string TranslateFileName = "translate.json";

        /// <summary>The installed language's font configuration, below <see cref="LayoutFolder"/>.</summary>
        public const string FontConfigFileName = "fontconfig.json";

        public static string FontsDirectory(string assetsRoot) => Path.Combine(assetsRoot, InterfaceFolder, FontsFolder);


        public static string PipboyDirectory(string assetsRoot) => Path.Combine(assetsRoot, InterfaceFolder, PipboyFolder);

        public static string PipboyArtManifestFile(string assetsRoot) => Path.Combine(PipboyDirectory(assetsRoot), PipboyArtManifestFileName);

        public static string UiLayoutDirectory(string assetsRoot) => Path.Combine(assetsRoot, InterfaceFolder, LayoutFolder);

    }
}
