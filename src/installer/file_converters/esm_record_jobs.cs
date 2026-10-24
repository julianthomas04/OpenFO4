using Dapper;
using Microsoft.Data.Sqlite;
using Mutagen.Bethesda;
using Mutagen.Bethesda.Fallout4;
using Mutagen.Bethesda.Plugins;
using Mutagen.Bethesda.Plugins.Records;

namespace AssetConverter
{
    // Raw-ESM scans Mutagen cannot provide for FO4: XCRI precombined refs and
    // the XMSP / MODS+MODC material-swap links, the XWCN/XWCU water currents and
    // the depth of field mode and vignette of the IMGS DNAM.
    internal sealed record RawScans(
        HashSet<string> Precombined,
        Dictionary<string, string> RefrSwaps,
        Dictionary<string, string> WaterCurrents,
        List<(string FormKey, string? SwapFormKey, float? ColorRemapIndex)> BaseSwaps,
        Dictionary<string, ImageSpaceDofBlock> ImageSpaceDepthOfField);

    internal sealed record ImportContext(IFallout4ModGetter File, Task<RawScans>? RawScans);

    // One unit of ESM import work: the processor(s) filling Tables, plus the
    // records-table rows (RecordTypes) it owns. Run writes through the given
    // connection; a null transaction lets the processor manage its own.
    internal sealed record RecordJob(
        string[] Tables,
        string[] RecordTypes,
        Func<ESMLoader, ImportContext, SqliteConnection, SqliteTransaction?, Task> Run,
        bool NeedsRawScans = false)
    {
        // The name a job goes by in timing reports: its record types, or its tables when it owns no records rows.
        internal string Label => string.Join('+', RecordTypes.Length > 0 ? RecordTypes : Tables);
    }

    // ADDN: BSValueNode attachment points. node_index is UNIQUE, so inserting a
    // later record replaces the earlier one with the same index (load order).
    internal sealed class AddonNodeProcessor : BaseRecordProcessor<IAddonNodeGetter>
    {
        private readonly Dictionary<int, int> _recordsPerNodeIndex = new();

        protected override async Task ProcessSpecificRecordAsync(IAddonNodeGetter record, string formKey, SqliteConnection conn, SqliteTransaction transaction)
        {
            const string sql = @"INSERT OR REPLACE INTO addon_nodes (form_key, editor_id, model_path, node_index, light, sound, master_particle_system_cap, flags)
                VALUES (@FormKey, @EditorId, @ModelPath, @NodeIndex, @Light, @Sound, @Cap, @Flags)";
            int? nodeIndex = record.NodeIndex;
            if (nodeIndex is int index)
                _recordsPerNodeIndex[index] = _recordsPerNodeIndex.GetValueOrDefault(index) + 1;
            await conn.ExecuteAsync(sql, new
            {
                FormKey = formKey,
                EditorId = record.EditorID,
                ModelPath = record.Model?.File,
                NodeIndex = nodeIndex,
                Light = record.Light.IsNull ? null : record.Light.FormKey.ToString(),
                Sound = record.Sound.IsNull ? null : record.Sound.FormKey.ToString(),
                Cap = (int?)record.MasterParticleSystemCap,
                Flags = (int)record.Flags
            }, transaction);
        }

        protected override void PrintSummary()
        {
            var duplicates = _recordsPerNodeIndex.Where(entry => entry.Value > 1).ToList();
            if (duplicates.Count == 0) return;
            Console.WriteLine($"  addon_nodes: {duplicates.Sum(entry => entry.Value - 1)} duplicate record(s) across {duplicates.Count} node_index value(s); the later record in load order wins");
        }
    }

    public partial class ESMLoader
    {
        private static RawScans ScanRawEsm(string path)
        {
            HashSet<string> precombined;
            using (EsmTiming.Measure("rawscan.xcri"))
                precombined = CellProcessor.ScanXcriFromEsm(path);
            Dictionary<string, string> refrSwaps;
            List<(string FormKey, string? SwapFormKey, float? ColorRemapIndex)> baseSwaps;
            using (EsmTiming.Measure("rawscan.material-swap-links"))
                (refrSwaps, baseSwaps) = CellProcessor.ScanMaterialSwapLinks(path);
            Dictionary<string, string> waterCurrents;
            using (EsmTiming.Measure("rawscan.water-currents"))
                waterCurrents = WaterCurrentScan.Scan(path);
            Dictionary<string, ImageSpaceDofBlock> depthOfField;
            using (EsmTiming.Measure("rawscan.image-space-dof"))
                depthOfField = ImageSpaceDofScan.Scan(path);
            return new RawScans(precombined, refrSwaps, waterCurrents, baseSwaps, depthOfField);
        }

        private static RecordJob Job<T>(string recordType, string[] tables,
            Func<IFallout4ModGetter, IEnumerable<T>> records, Func<ESMLoader, IRecordProcessor<T>> processor,
            bool needsRawScans = false)
            where T : IMajorRecordGetter =>
            new(tables, [recordType], (loader, context, conn, transaction) =>
                loader.ProcessGroupAsync(records(context.File), recordType, conn, processor(loader), transaction),
                needsRawScans);

        private static RecordJob JobWithScans<T>(string recordType, string[] tables,
            Func<IFallout4ModGetter, IEnumerable<T>> records, Func<RawScans, IRecordProcessor<T>> processor)
            where T : IMajorRecordGetter =>
            new(tables, [recordType], async (loader, context, conn, transaction) =>
            {
                var scans = await (context.RawScans
                    ?? throw new InvalidOperationException("Job requires raw ESM scans but the import plan did not schedule them."));
                await loader.ProcessGroupAsync(records(context.File), recordType, conn, processor(scans), transaction);
            }, NeedsRawScans: true);

        private static RecordJob Simple<T>(string recordType, string table,
            Func<IFallout4ModGetter, IEnumerable<T>> records)
            where T : IMajorRecordGetter =>
            Job<T>(recordType, [table], records, _ => new SimpleRecordProcessor<T>(table));

        internal static readonly RecordJob PerkJob = new(["perks", "perk_chart"], ["PERK"],
            (loader, context, conn, transaction) => loader.ProcessPerksAsync(context.File, conn, transaction));

        internal static readonly RecordJob ActorValueJob = new(["actor_value_informations", "special_attributes"], ["AVIF"],
            (loader, context, conn, transaction) => loader.ProcessActorValuesAsync(context.File, conn, transaction));

        // Record groups with no dependencies on each other: the full import
        // runs them concurrently, a partial reload runs the selected ones in turn.
        internal static readonly IReadOnlyList<RecordJob> RecordJobs =
        [
        Job<IGameSettingGetter>("GMST", ["game_settings"], f => f.GameSettings, _ => new GameSettingsProcessor()),
        Job<IGlobalGetter>("GLOB", ["globals"], f => f.Globals, _ => new GlobalsProcessor()),
        Job<IKeywordGetter>("KYWD", ["keywords"], f => f.Keywords, _ => new KeywordProcessor()),
        Job<IRaceGetter>("RACE", ["races"], f => f.Races, _ => new RaceProcessor()),
        Job<INpcGetter>("NPC_", ["npcs"], f => f.Npcs, _ => new NpcProcessor()),
        Job<IWeaponGetter>("WEAP", ["items", "weapons"], f => f.Weapons, _ => new WeaponProcessor()),
        Job<IArmorGetter>("ARMO", ["items", "armors"], f => f.Armors, _ => new ArmorProcessor()),
        Job<IAmmunitionGetter>("AMMO", ["items"], f => f.Ammunitions, _ => new AmmoProcessor()),
        Job<IMiscItemGetter>("MISC", ["items"], f => f.MiscItems, _ => new MiscItemProcessor()),
        Job<IBookGetter>("BOOK", ["items"], f => f.Books, _ => new BookProcessor()),
        Job<IIngestibleGetter>("ALCH", ["items"], f => f.Ingestibles, _ => new IngestibleProcessor()),
        Job<IStaticGetter>("STAT", ["statics"], f => f.Statics, _ => new StaticProcessor()),
        Job<IActivatorGetter>("ACTI", ["activators"], f => f.Activators, _ => new ActivatorProcessor()),
        Job<IContainerGetter>("CONT", ["containers"], f => f.Containers, _ => new ContainerProcessor()),
        Job<IDoorGetter>("DOOR", ["doors"], f => f.Doors, _ => new DoorProcessor()),
        Job<IFurnitureGetter>("FURN", ["furniture"], f => f.Furniture, _ => new FurnitureProcessor()),
        Job<ILightGetter>("LIGH", ["lights"], f => f.Lights, _ => new LightProcessor()),
        Job<IMovableStaticGetter>("MSTT", ["movable_statics"], f => f.MovableStatics, _ => new MovableStaticProcessor()),
        Job<ITalkingActivatorGetter>("TACT", ["talking_activators"], f => f.TalkingActivators, _ => new TalkingActivatorProcessor()),
        Job<IStaticCollectionGetter>("SCOL", ["static_collections"], f => f.StaticCollections, _ => new StaticCollectionProcessor()),
        Job<IFactionGetter>("FACT", ["factions"], f => f.Factions, _ => new FactionProcessor()),
        Job<IEncounterZoneGetter>("ECZN", ["encounter_zones"], f => f.EncounterZones, _ => new EncounterZoneProcessor()),
        Job<ILocationGetter>("LCTN", ["locations"], f => f.Locations, _ => new LocationProcessor()),
        Job<IFormListGetter>("FLST", ["form_lists"], f => f.FormLists, _ => new FormListProcessor()),
        Job<IAObjectModificationGetter>("OMOD", ["object_modifications"], f => f.EnumerateMajorRecords<IAObjectModificationGetter>(), _ => new ObjectModificationProcessor()),
        Job<IQuestGetter>("QUST", ["quests"], f => f.Quests, _ => new QuestProcessor()),
        Job<IMagicEffectGetter>("MGEF", ["magic_effects"], f => f.MagicEffects, _ => new MagicEffectProcessor()),
        Job<ISpellGetter>("SPEL", ["spells"], f => f.Spells, _ => new SpellProcessor()),
        PerkJob,
        Job<IWeatherGetter>("WTHR", ["weather"], f => f.EnumerateMajorRecords<IWeatherGetter>(), _ => new WeatherProcessor()),
        Job<ISoundDescriptorGetter>("SNDR", ["sound_descriptors", "sound_descriptor_children", "sound_descriptor_conditions"], f => f.SoundDescriptors, _ => new SoundDescriptorProcessor()),
        Job<IMusicTrackGetter>("MUST", ["music_tracks", "music_track_layers", "music_track_conditions"], f => f.MusicTracks, _ => new MusicTrackProcessor()),
        Job<IMaterialObjectGetter>("MATO", ["material_objects"], f => f.EnumerateMajorRecords<IMaterialObjectGetter>(), _ => new MaterialObjectProcessor()),
        Job<ILeveledItemGetter>("LVLI", ["leveled_items"], f => f.LeveledItems, _ => new LeveledItemProcessor()),
        Job<ILeveledNpcGetter>("LVLN", ["leveled_npcs"], f => f.LeveledNpcs, _ => new LeveledNpcProcessor()),
        Job<IConstructibleObjectGetter>("COBJ", ["constructible_objects"], f => f.ConstructibleObjects, _ => new ConstructibleObjectProcessor()),
        Job<IComponentGetter>("CMPO", ["components"], f => f.Components, _ => new ComponentProcessor()),
        Job<IFloraGetter>("FLOR", ["flora"], f => f.EnumerateMajorRecords<IFloraGetter>(), _ => new FloraProcessor()),
        Job<ITreeGetter>("TREE", ["trees"], f => f.Trees, _ => new TreeProcessor()),
        Job<IGrassGetter>("GRAS", ["grasses"], f => f.Grasses, _ => new GrassProcessor()),
        Job<IClassGetter>("CLAS", ["classes"], f => f.Classes, _ => new ClassProcessor()),
        Job<ICombatStyleGetter>("CSTY", ["combat_styles"], f => f.CombatStyles, _ => new CombatStyleProcessor()),
        Job<IEquipTypeGetter>("EQUP", ["equip_types"], f => f.EquipTypes, _ => new EquipTypeProcessor()),
        Job<IVoiceTypeGetter>("VTYP", ["voice_types"], f => f.VoiceTypes, _ => new VoiceTypeProcessor()),
        Job<IExplosionGetter>("EXPL", ["explosions"], f => f.Explosions, _ => new ExplosionProcessor()),
        Job<IProjectileGetter>("PROJ", ["projectiles"], f => f.Projectiles, _ => new ProjectileProcessor()),
        Job<IHazardGetter>("HAZD", ["hazards"], f => f.Hazards, _ => new HazardProcessor()),
        Job<IIngredientGetter>("INGR", ["items"], f => f.Ingredients, _ => new IngredientProcessor()),
        Job<IKeyGetter>("KEYM", ["items"], f => f.Keys, _ => new KeyProcessor()),
        Job<IHolotapeGetter>("HOLT", ["items"], f => f.Holotapes, _ => new HolotapeProcessor()),
        Job<IOutfitGetter>("OTFT", ["outfits"], f => f.Outfits, _ => new OutfitProcessor()),
        Job<IRelationshipGetter>("RELA", ["relationships"], f => f.Relationships, _ => new RelationshipProcessor()),
        Job<IObjectEffectGetter>("ENCH", ["object_effects"], f => f.ObjectEffects, _ => new ObjectEffectProcessor()),
        Job<ILightingTemplateGetter>("LGTM", ["lighting_templates"], f => f.LightingTemplates, _ => new LightingTemplateProcessor()),
        JobWithScans<IImageSpaceGetter>("IMGS", ["image_spaces"], f => f.ImageSpaces, scans => new ImageSpaceProcessor(scans.ImageSpaceDepthOfField)),
        Job<IVisualEffectGetter>("RFCT", ["visual_effects"], f => f.VisualEffects, _ => new VisualEffectProcessor()),
        Job<IWaterGetter>("WATR", ["waters"], f => f.Waters, _ => new WaterProcessor()),
        Job<IClimateGetter>("CLMT", ["climates"], f => f.Climates, _ => new ClimateProcessor()),
        Job<IHeadPartGetter>("HDPT", ["head_parts"], f => f.HeadParts, _ => new HeadPartProcessor()),
        Job<IPackageGetter>("PACK", ["packages"], f => f.Packages, _ => new PackageProcessor()),
        Job<ISoundCategoryGetter>("SCAT", ["sound_categories"], f => f.SoundCategories, _ => new SoundCategoryProcessor()),
        Job<ISoundOutputModelGetter>("SOPM", ["sound_output_models", "sound_output_channels"], f => f.SoundOutputModels, _ => new SoundOutputModelProcessor()),
        Job<IRegionGetter>("REGN", ["regions"], f => f.Regions, _ => new RegionProcessor()),
        Job<IMessageGetter>("MESG", ["messages"], f => f.Messages, _ => new MessageProcessor()),
        Job<ITerminalGetter>("TERM", ["terminals"], f => f.Terminals, _ => new TerminalProcessor()),
        Job<IMovementTypeGetter>("MOVT", ["movement_types"], f => f.MovementTypes, _ => new MovementTypeProcessor()),
        Job<ILandscapeTextureGetter>("LTEX", ["landscape_textures", "ltex_grasses"], f => f.EnumerateMajorRecords<ILandscapeTextureGetter>(), _ => new LandscapeTextureProcessor()),
        Job<ITextureSetGetter>("TXST", ["texture_sets"], f => f.TextureSets, l => new TextureSetProcessor(l._materialsRoot)),
        Simple<IAcousticSpaceGetter>("ASPC", "acoustic_spaces", f => f.AcousticSpaces),
        Simple<IActionRecordGetter>("AACT", "action_records", f => f.EnumerateMajorRecords<IActionRecordGetter>()),
        ActorValueJob,
        Job<IAddonNodeGetter>("ADDN", ["addon_nodes"], f => f.AddonNodes, _ => new AddonNodeProcessor()),
        Simple<IAimModelGetter>("AMDL", "aim_models", f => f.AimModels),
        Simple<IAnimatedObjectGetter>("ANIO", "animated_objects", f => f.AnimatedObjects),
        Simple<IAnimationSoundTagSetGetter>("ASTS", "animation_sound_tag_sets", f => f.AnimationSoundTagSets),
        Simple<IArmorAddonGetter>("ARMA", "armor_addons", f => f.ArmorAddons),
        Simple<IArtObjectGetter>("ARTO", "art_objects", f => f.ArtObjects),
        Simple<IAssociationTypeGetter>("ASTP", "association_types", f => f.AssociationTypes),
        Simple<IAttractionRuleGetter>("AIRU", "attraction_rules", f => f.AttractionRules),
        Simple<IAudioCategorySnapshotGetter>("SNCP", "audio_category_snapshots", f => f.AudioCategorySnapshots),
        Simple<IAudioEffectChainGetter>("AEIC", "audio_effect_chains", f => f.AudioEffectChains),
        Simple<IBendableSplineGetter>("BNDS", "bendable_splines", f => f.BendableSplines),
        Simple<IBodyPartDataGetter>("BPTD", "body_part_datas", f => f.EnumerateMajorRecords<IBodyPartDataGetter>()),
        Simple<ICameraPathGetter>("CPTH", "camera_paths", f => f.CameraPaths),
        Simple<ICameraShotGetter>("CAMS", "camera_shots", f => f.CameraShots),
        Job<ICollisionLayerGetter>("COLL", ["collision_layers"], f => f.CollisionLayers, _ => new CollisionLayerProcessor()),
        Simple<IColorRecordGetter>("CLFM", "color_records", f => f.EnumerateMajorRecords<IColorRecordGetter>()),
        Simple<IDebrisGetter>("DEBR", "debrises", f => f.EnumerateMajorRecords<IDebrisGetter>()),
        Job<IDefaultObjectManagerGetter>("DOMD", ["default_object_managers", "default_object_slots"], f => f.DefaultObjectManagers, _ => new DefaultObjectManagerProcessor()),
        Simple<IDefaultObjectGetter>("DOBJ", "default_objects", f => f.DefaultObjects),
        Simple<IDialogBranchGetter>("DLBR", "dialog_branches", f => f.EnumerateMajorRecords<IDialogBranchGetter>()),
        Simple<IDialogResponsesGetter>("INFO", "dialog_responseses", f => f.EnumerateMajorRecords<IDialogResponsesGetter>()),
        Simple<IDialogTopicGetter>("DIAL", "dialog_topics", f => f.EnumerateMajorRecords<IDialogTopicGetter>()),
        Simple<IDialogViewGetter>("DLVW", "dialog_views", f => f.DialogViews),
        Simple<IDualCastDataGetter>("DUAL", "dual_cast_datas", f => f.EnumerateMajorRecords<IDualCastDataGetter>()),
        Simple<IEffectShaderGetter>("EFSH", "effect_shaders", f => f.EffectShaders),
        Job<IFootstepSetGetter>("FSTS", ["footstep_sets", "footstep_set_entries"], f => f.FootstepSets, _ => new FootstepSetProcessor()),
        Job<IFootstepGetter>("FSTP", ["footsteps"], f => f.Footsteps, _ => new FootstepProcessor()),
        Simple<IGodRaysGetter>("GDRY", "god_rayses", f => f.EnumerateMajorRecords<IGodRaysGetter>()),
        Simple<IIdleAnimationGetter>("IDLE", "idle_animations", f => f.IdleAnimations),
        Simple<IIdleMarkerGetter>("IDLM", "idle_markers", f => f.IdleMarkers),
        Simple<IImageSpaceAdapterGetter>("IMAD", "image_space_adapters", f => f.ImageSpaceAdapters),
        Job<IImpactDataSetGetter>("IPDS", ["impact_data_sets", "impact_data_set_entries"], f => f.ImpactDataSets, _ => new ImpactDataSetProcessor()),
        Job<IImpactGetter>("IPCT", ["impacts"], f => f.Impacts, _ => new ImpactProcessor()),
        Simple<IInstanceNamingRulesGetter>("INNR", "instance_naming_ruleses", f => f.EnumerateMajorRecords<IInstanceNamingRulesGetter>()),
        Simple<ILayerGetter>("LAYR", "layers", f => f.Layers),
        Simple<ILensFlareGetter>("LNSF", "lens_flares", f => f.LensFlares),
        Simple<ILoadScreenGetter>("LSCR", "load_screens", f => f.LoadScreens),
        Simple<ILocationReferenceTypeGetter>("LCRT", "location_reference_types", f => f.LocationReferenceTypes),
        Job<IMaterialSwapGetter>("MSWP", ["material_swaps", "material_swap_entries", "base_material_swaps"], f => f.MaterialSwaps, l => new MaterialSwapProcessor(l._materialsRoot, l._texturesOut), needsRawScans: true),
        Job<IMaterialTypeGetter>("MATT", ["material_types"], f => f.MaterialTypes, _ => new MaterialTypeProcessor()),
        Job<IMusicTypeGetter>("MUSC", ["music_types", "music_type_tracks"], f => f.MusicTypes, _ => new MusicTypeProcessor()),
        Simple<INavigationMeshInfoMapGetter>("NAVI", "navigation_mesh_info_maps", f => f.NavigationMeshInfoMaps),
        Simple<INavigationMeshObstacleManagerGetter>("NVOM", "navigation_mesh_obstacle_managers", f => f.NavigationMeshObstacleManagers),
        Simple<IObjectVisibilityManagerGetter>("OBJE", "object_visibility_managers", f => f.ObjectVisibilityManagers),
        Simple<IPackInGetter>("PCBE", "pack_ins", f => f.PackIns),
        Simple<IReferenceGroupGetter>("RFGP", "reference_groups", f => f.ReferenceGroups),
        Simple<IReverbParametersGetter>("REVB", "reverb_parameterses", f => f.EnumerateMajorRecords<IReverbParametersGetter>()),
        Simple<ISceneCollectionGetter>("STSC", "scene_collections", f => f.SceneCollections),
        Simple<ISceneGetter>("SCEN", "scenes", f => f.EnumerateMajorRecords<ISceneGetter>()),
        Simple<IShaderParticleGeometryGetter>("SPGD", "shader_particle_geometries", f => f.ShaderParticleGeometries),
        Simple<ISoundKeywordMappingGetter>("SKMA", "sound_keyword_mappings", f => f.SoundKeywordMappings),
        Simple<ISoundMarkerGetter>("SOUN", "sound_markers", f => f.SoundMarkers),
        Simple<ITransformGetter>("TRNS", "transforms", f => f.Transforms),
        Simple<IZoomGetter>("ZOOM", "zooms", f => f.Zooms),
        Simple<IADamageTypeGetter>("DMGT", "damage_types", f => f.EnumerateMajorRecords<IADamageTypeGetter>()),
        Simple<IAStoryManagerNodeGetter>("SMBN", "story_manager_nodes", f => f.EnumerateMajorRecords<IAStoryManagerNodeGetter>()),
        NavMeshJob(),
        CellPlacesJob(),
        LocationMusicJob(),
        WorldspaceMusicJob(),
        RegionMusicJob(),
        ];

        internal static readonly RecordJob WorldspaceJob = new(["worldspaces", "map_markers"], ["WRLD"],
            (loader, context, conn, transaction) =>
                loader.ProcessWorldspaceRecordsAsync(context.File.Worldspaces, conn, transaction));

        internal static readonly RecordJob CellsJob = new(
            ["cells", "placed_objects", "placed_teleports", "landscapes"], ["CELL", "REFR", "LAND"],
            async (loader, context, conn, transaction) =>
            {
                var scans = await (context.RawScans
                    ?? throw new InvalidOperationException("Job requires raw ESM scans but the import plan did not schedule them."));
                await loader.ProcessCellsAsync(context.File, conn, transaction, scans.Precombined, scans.RefrSwaps, scans.WaterCurrents);
            },
            NeedsRawScans: true);

        internal static readonly IReadOnlyList<RecordJob> AllJobs = [.. RecordJobs, WorldspaceJob, CellsJob];
    }
}
