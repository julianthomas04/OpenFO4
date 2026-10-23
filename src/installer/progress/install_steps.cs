namespace AssetConverter.Progress
{
    /// <summary>
    /// The work of the full install, declared up front so the overall progress is known from the start. Phases opened through
    /// <see cref="InstallProgress.BeginPhase"/> bind to a declared unit by name; units that share a group are shown as one step
    /// (<see cref="StepDefinition.Group"/>) while each keeps running in its own lane. Weights are seconds of work per unit measured
    /// on a cold full install (converting shaders: its uncontended estimate; building the database: its own work without the waits
    /// on gates); the end-of-run summary lists the real durations to recalibrate them from.
    /// </summary>
    public static class InstallSteps
    {
        public const string Initializing = "Initializing";
        public const string Finalizing = "Finalizing";

        public const string UnpackPayload = "Unpacking installer files";
        public const string InstallGame = "Installing the game";
        public const string ExtractMeshes = "Extracting meshes and materials";
        public const string CopyGameData = "Copying game data";
        public const string ExtractInterface = "Extracting interface assets";
        public const string ConvertShaders = "Converting shaders";
        public const string ConvertAudio = "Converting audio";
        public const string ConvertVideo = "Converting videos";
        public const string ExtractTextures = "Extracting textures";
        public const string ConvertMeshes = "Converting meshes";
        public const string ExtractCollision = "Extracting collision";
        public const string ConvertAnimations = "Converting animations";
        public const string BuildDatabase = "Building database";
        public const string BakePalettes = "Baking palette textures";
        public const string ScanLodWater = "Scanning terrain LOD water";
        public const string FinalizeDatabase = "Finalizing database";
        public const string RemoveStaging = "Cleaning up";

        public static IReadOnlyList<StepDefinition> All { get; } =
        [
            new(InstallGame, "files", Weight: 1, Group: Initializing),
            new(CopyGameData, "items", Weight: 7, Group: Initializing),
            new(ExtractMeshes, "files", Weight: 89),
            new(ExtractInterface, "items", Weight: 7),
            new(ConvertShaders, "shaders", Weight: 10),
            new(ConvertAudio, "files", Weight: 60),
            new(ConvertVideo, "videos", Weight: 600),
            new(ExtractTextures, "files", Weight: 195),
            new(ConvertMeshes, "meshes", Weight: 165),
            new(ExtractCollision, "meshes", Weight: 50),
            new(ConvertAnimations, "creatures", Weight: 96),
            new(BuildDatabase, "records", Weight: 55),
            new(BakePalettes, "tasks", Weight: 9, Group: Finalizing),
            new(ScanLodWater, "tasks", Weight: 2, Group: Finalizing),
            new(FinalizeDatabase, "tasks", Weight: 13, Group: Finalizing),
            new(RemoveStaging, "items", Weight: 15),
        ];

        /// <summary>Unpacking the payload comes before everything else and exists only for a packaged installer, which carries one; it is part of initializing.</summary>
        public static StepDefinition UnpackPayloadStep { get; } = new(UnpackPayload, "files", Weight: 10, Group: Initializing);

        /// <summary>The names of the steps as shown for the given declared work, in order.</summary>
        public static IReadOnlyList<string> VisibleNames(IEnumerable<StepDefinition> work) =>
            [.. work.Select(unit => unit.VisibleName).Distinct(StringComparer.OrdinalIgnoreCase)];

        /// <summary>The work of one install: all of it, except the removal of the temporary files when they are kept, preceded by the unpacking of the payload when the installer has one.</summary>
        public static IReadOnlyList<StepDefinition> Planned(bool keepStaging, bool unpacksPayload = false)
        {
            IEnumerable<StepDefinition> steps = keepStaging ? All.Where(step => step.Name != RemoveStaging) : All;
            return [.. unpacksPayload ? steps.Prepend(UnpackPayloadStep) : steps];
        }

        /// <summary>Runs before every other lane, on its own.</summary>
        public static IReadOnlyList<string> UnpackLane { get; } = [UnpackPayload];

        public static IReadOnlyList<string> GameLane { get; } = [InstallGame];

        public static IReadOnlyList<string> PumpLane { get; } = [ExtractMeshes, CopyGameData, ExtractTextures];

        public static IReadOnlyList<string> InterfaceLane { get; } = [ExtractInterface];

        public static IReadOnlyList<string> ShaderLane { get; } = [ConvertShaders];

        public static IReadOnlyList<string> AudioLane { get; } = [ConvertAudio];

        public static IReadOnlyList<string> VideoLane { get; } = [ConvertVideo];

        public static IReadOnlyList<string> MeshLane { get; } = [ConvertMeshes];

        public static IReadOnlyList<string> CollisionLane { get; } = [ExtractCollision];

        public static IReadOnlyList<string> AnimationLane { get; } = [ConvertAnimations];

        public static IReadOnlyList<string> DatabaseLane { get; } = [BuildDatabase];

        public static IReadOnlyList<string> PaletteLane { get; } = [BakePalettes];

        public static IReadOnlyList<string> LodWaterLane { get; } = [ScanLodWater];

        public static IReadOnlyList<string> TailLane { get; } = [FinalizeDatabase];

        /// <summary>Runs after every other lane has succeeded.</summary>
        public static IReadOnlyList<string> CleanupLane { get; } = [RemoveStaging];

        /// <summary>Every concurrent lane of the install; each declared unit of work belongs to exactly one.</summary>
        public static IReadOnlyList<IReadOnlyList<string>> Lanes { get; } =
            [GameLane, PumpLane, InterfaceLane, ShaderLane, AudioLane, VideoLane, MeshLane, CollisionLane, AnimationLane, DatabaseLane, PaletteLane, LodWaterLane, TailLane, CleanupLane];
    }
}
