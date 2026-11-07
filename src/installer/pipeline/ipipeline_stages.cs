namespace AssetConverter.Pipeline
{
    /// <summary>
    /// The work of the stages of the full install, as <see cref="InstallPipeline"/> calls it: the pipeline decides what runs, when and after
    /// what, the stages do the work. <see cref="PipelineStages"/> is the real thing; a test of the pipeline's own wiring (which lanes run
    /// for which asset groups, which gates open, what the marker lists) substitutes stages that only record what was asked of them.
    /// </summary>
    internal interface IPipelineStages
    {
        /// <summary>Points the converters' process-wide settings at this install and lets the database load defer the palette bakes it finds; undone when the result is disposed.</summary>
        IDisposable ConfigureConverters();

        void InstallGame(GameFiles game);

        /// <returns>Whether every entry of every archive extracted cleanly; false leaves the archives that failed without a done-marker.</returns>
        bool ExtractMeshArchives(NifWorkQueue meshes, PriorityFiles skeletons, Action onInputsReady);

        TextureArchiveSet OpenAndIndexTextureArchives();

        void CopyGameData(Action onEsmCopied);

        void CopyGameInis();

        /// <returns>Whether every texture archive extracted cleanly.</returns>
        bool ExtractTextures(TextureArchiveSet textures);

        void ExtractColourLuts();

        void TranslateShaders();

        void ExtractInterfaceAssets();

        void ConvertAudio();

        void ConvertVideo();

        void ConvertMeshes(NifWorkQueue meshes, PriorityFiles skeletons);

        void QueueUnobservedCollisionCandidates();

        void ExtractCollision();

        void ConvertAnimations();

        void BakePalettes();

        Task LoadDatabase();

        void ScanLodWater();

        /// <summary>Needs <see cref="ConfigureConverters"/> and <see cref="ScanLodWater"/> to have run.</summary>
        void FinalizeDatabase(Task databaseLoaded);

        StagingRemovalResult RemoveStaging();
    }
}
