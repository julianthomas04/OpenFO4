using AssetConverter;
using AssetConverter.Pipeline;
using Dapper;
using Microsoft.Data.Sqlite;
using Mutagen.Bethesda.Fallout4;
using Mutagen.Bethesda.Plugins;
using Noggog;
using Xunit;

namespace InstallerTests
{
    // The sound and footstep tables. A real binary plugin built from Mutagen's
    // in-memory records is loaded through the real loader.
    public sealed class EsmLoadSoundTests : IDisposable
    {
        const double PercentByte = 255.0;
        const int FirstPersonCondition = 391;

        readonly TempDirectory _directory = new();
        readonly string _dbPath;

        public EsmLoadSoundTests() => _dbPath = Path.Combine(_directory.Path, "data.db");

        public void Dispose()
        {
            SqliteConnection.ClearAllPools();
            _directory.Dispose();
        }

        async Task<SqliteConnection> LoadAsync(TinyPlugin plugin)
        {
            string esmPath = plugin.Write(_directory.Path);
            await new ESMLoader(_dbPath) { Memory = MemoryBudget.Unlimited() }.LoadESMAsync(esmPath, ct: TestContext.Current.CancellationToken);
            var connection = new SqliteConnection(new SqliteConnectionStringBuilder { DataSource = _dbPath, Pooling = false }.ToString());
            connection.Open();
            return connection;
        }

        static FormLink<ISoundDescriptorGetter> Link(SoundDescriptor descriptor) => new(descriptor.FormKey);

        static SoundDescriptorStandardData Standard(double shiftByte, double varianceByte, sbyte priority, sbyte dbVariance, float attenuation) => new()
        {
            PercentFrequencyShift = new Percent(shiftByte / PercentByte),
            PercentFrequencyVariance = new Percent(varianceByte / PercentByte),
            Priority = priority,
            Variance = dbVariance,
            StaticAttenuation = attenuation,
        };

        [Fact]
        public async Task StandardDescriptor_StoresTheBnamFieldsAsTheRecordSpellsThem()
        {
            var plugin = new TinyPlugin();
            plugin.AddSoundDescriptor("StepLayer", descriptor =>
            {
                descriptor.Data = Standard(10, 4, unchecked((sbyte)192), 6, 3.52f);
                descriptor.SoundFiles.AddRange([@"data\Sound\FX\step_01.wav", @"Data\Sound\FX\step_02.wav"]);
                descriptor.LoopAndRumble = new SoundLoopAndRumble { Unknown = 1, Loop = SoundDescriptor.LoopType.None };
            });

            using var connection = await LoadAsync(plugin);
            var row = connection.QuerySingle("SELECT * FROM sound_descriptors WHERE form_key = @key", new { key = plugin["StepLayer"] });

            Assert.Equal((long)SoundDescriptorMapper.StandardType, row.descriptor_type);
            Assert.Equal((10L, 4L, 192L, 6L, 352L), (row.freq_shift_pct, row.freq_variance_pct, row.priority, row.db_variance, row.static_attenuation_centidb));
            Assert.Null(row.base_descriptor);
            Assert.Equal(1L, row.looping_flags);
            Assert.Equal("""["data\\Sound\\FX\\step_01.wav","Data\\Sound\\FX\\step_02.wav"]""", row.sound_files);
        }

        [Fact]
        public async Task StandardDescriptor_KeepsNegativeFrequencyShiftSigned()
        {
            var plugin = new TinyPlugin();
            plugin.AddSoundDescriptor("Lower", descriptor => descriptor.Data = Standard(248, 0, 0, 0, 0f));

            using var connection = await LoadAsync(plugin);

            Assert.Equal(-8L, connection.ExecuteScalar<long>("SELECT freq_shift_pct FROM sound_descriptors WHERE form_key = @key", new { key = plugin["Lower"] }));
        }

        [Fact]
        public async Task AutoweaponDescriptor_StoresTheBaseDescriptorAndNoStandardFields()
        {
            var plugin = new TinyPlugin();
            var baseDescriptor = plugin.AddSoundDescriptor("Base", descriptor => descriptor.Data = Standard(0, 0, 0, 0, 0f));
            plugin.AddSoundDescriptor("Auto", descriptor => descriptor.Data = new SoundDescriptorAutoweaponData { BaseDescriptor = Link(baseDescriptor) });

            using var connection = await LoadAsync(plugin);
            var row = connection.QuerySingle("SELECT * FROM sound_descriptors WHERE form_key = @key", new { key = plugin["Auto"] });

            Assert.Equal((long)SoundDescriptorMapper.AutoweaponType, row.descriptor_type);
            Assert.Equal(plugin["Base"], row.base_descriptor);
            Assert.Null(row.freq_shift_pct);
            Assert.Null(row.static_attenuation_centidb);
        }

        [Fact]
        public async Task CompoundDescriptor_KeepsChildrenInOrderAndTheirConditions()
        {
            var plugin = new TinyPlugin();
            var first = plugin.AddSoundDescriptor("FirstPerson", descriptor =>
            {
                descriptor.Data = Standard(0, 0, 0, 0, 0f);
                descriptor.Conditions.Add(new ConditionFloat
                {
                    Data = new FunctionConditionData { Function = (Condition.Function)FirstPersonCondition },
                    CompareOperator = CompareOperator.EqualTo,
                    ComparisonValue = 1f,
                });
            });
            var third = plugin.AddSoundDescriptor("ThirdPerson", descriptor => descriptor.Data = Standard(0, 0, 0, 0, 0f));
            plugin.AddSoundDescriptor("Compound", descriptor =>
            {
                descriptor.Data = new SoundDescriptorCompoundData();
                descriptor.Descriptors.AddRange([Link(third), Link(first)]);
            });

            using var connection = await LoadAsync(plugin);
            var children = connection.Query<(long Ord, string Child)>("SELECT ord, child FROM sound_descriptor_children WHERE descriptor = @key ORDER BY ord", new { key = plugin["Compound"] }).ToList();
            var condition = connection.QuerySingle("SELECT * FROM sound_descriptor_conditions WHERE descriptor = @key", new { key = plugin["FirstPerson"] });

            Assert.Equal([(0L, plugin["ThirdPerson"]), (1L, plugin["FirstPerson"])], children);
            Assert.Equal(((long)FirstPersonCondition, 0L, 1.0, 0L), (condition.function, condition.compare_op, condition.compare_value, condition.or_flag));
            Assert.Equal((long)SoundDescriptorMapper.CompoundType, connection.ExecuteScalar<long>("SELECT descriptor_type FROM sound_descriptors WHERE form_key = @key", new { key = plugin["Compound"] }));
        }

        [Fact]
        public void SoundDescriptorMapper_RejectsADescriptorWithoutData()
        {
            var plugin = new TinyPlugin();
            var descriptor = plugin.AddSoundDescriptor("NoData", _ => { });

            Assert.Throws<UnsupportedEsmDataException>(() => SoundDescriptorMapper.Map(descriptor, "subject"));
        }

        [Fact]
        public async Task OutputModel_StoresTypeAttenuationAndTheSpeakerMatrix()
        {
            var plugin = new TinyPlugin();
            plugin.AddSoundOutputModel("FirstPersonModel", model =>
            {
                model.Type = SoundOutputModel.TypeEnum.DefinedSpeakerOutput;
                model.StaticAttenuation = 1.76f;
                model.OutputChannels = new SoundOutputChannels
                {
                    Channel0 = new SoundOutputChannel { L = 75, R = 75, RL = 50, RR = 50, BL = 50, BR = 50 },
                    Channel1 = new SoundOutputChannel { L = 100, RL = 100, BL = 100 },
                    Channel2 = new SoundOutputChannel { R = 100, RR = 100, BR = 100 },
                };
            });
            plugin.AddSoundOutputModel("Hrtf", model => model.Type = SoundOutputModel.TypeEnum.UsesHrtf);

            using var connection = await LoadAsync(plugin);
            var model = connection.QuerySingle("SELECT output_type, static_attenuation_db FROM sound_output_models WHERE form_key = @key", new { key = plugin["FirstPersonModel"] });
            var percents = connection.Query<(long Ord, string Speaker, long Percent)>("SELECT ord, speaker, percent FROM sound_output_channels WHERE model = @key", new { key = plugin["FirstPersonModel"] }).ToList();

            Assert.Equal((long)SoundOutputModel.TypeEnum.DefinedSpeakerOutput, model.output_type);
            Assert.Equal(1.76, model.static_attenuation_db);
            Assert.Equal(24, percents.Count);
            Assert.Equal(75L, percents.Single(row => row.Ord == 0 && row.Speaker == "L").Percent);
            Assert.Equal(100L, percents.Single(row => row.Ord == 2 && row.Speaker == "BR").Percent);
            Assert.Equal(0L, percents.Single(row => row.Ord == 1 && row.Speaker == "R").Percent);
        }

        [Fact]
        public void FootstepSetGroups_ReadTheStoredArrayInReverseGroupOrder()
        {
            string[] stored = ["swim0", "swim1", "sneak0", "run0", "walk0", "walk1"];

            var rows = FootstepSetGroups.Split([2, 1, 0, 1, 2], stored, "subject");

            Assert.Equal(
                [("swim", 0, "swim0"), ("swim", 1, "swim1"), ("sneak", 0, "sneak0"), ("run", 0, "run0"), ("walk", 0, "walk0"), ("walk", 1, "walk1")],
                rows);
        }

        [Fact]
        public void FootstepSetGroups_RejectCountsThatDoNotMatchTheStoredArray()
        {
            string[] stored = ["a", "b", "c"];

            Assert.Throws<UnsupportedEsmDataException>(() => FootstepSetGroups.Split([1, 1, 0, 0, 0], stored, "subject"));
            Assert.Throws<UnsupportedEsmDataException>(() => FootstepSetGroups.Split([1, 1, 1, 0], stored, "subject"));
        }

        [Fact]
        public async Task FootstepSet_AssignsTheGaitsFromTheStoredOrder()
        {
            var plugin = new TinyPlugin();
            var stored = new[] { "SwimStep", "SneakStep", "RunStepA", "RunStepB", "WalkStep" }
                .Select(name => plugin.AddFootstep(name, step => step.Tag = name)).ToList();
            plugin.AddFootstepSet("Set", set =>
            {
                // Mutagen cuts the stored array by the counts in walk, run, sprint, sneak, swim order, so the lists below spell the
                // stored array: swim 1, sneak 1, run 2, walk 1.
                set.WalkFootsteps.Add(new FormLink<IFootstepGetter>(stored[0].FormKey));
                set.RunFootsteps.AddRange([new FormLink<IFootstepGetter>(stored[1].FormKey), new FormLink<IFootstepGetter>(stored[2].FormKey)]);
                set.SneakFootsteps.Add(new FormLink<IFootstepGetter>(stored[3].FormKey));
                set.SwimFootsteps.Add(new FormLink<IFootstepGetter>(stored[4].FormKey));
            });

            using var connection = await LoadAsync(plugin);
            var rows = connection.Query<(string Gait, long Ord, string Footstep)>("SELECT gait, ord, footstep FROM footstep_set_entries WHERE set_key = @key ORDER BY gait, ord", new { key = plugin["Set"] }).ToList();

            Assert.Equal(
                [("run", 0L, plugin["RunStepA"]), ("run", 1L, plugin["RunStepB"]), ("sneak", 0L, plugin["SneakStep"]), ("swim", 0L, plugin["SwimStep"]), ("walk", 0L, plugin["WalkStep"])],
                rows);
        }

        [Fact]
        public async Task Footstep_StoresItsTagAndImpactDataSet()
        {
            var plugin = new TinyPlugin();
            var impactSet = plugin.AddImpactDataSet("Impacts", _ => { });
            plugin.AddFootstep("RightFoot", step =>
            {
                step.Tag = "FootRight";
                step.ImpactDataSet = new FormLink<IImpactDataSetGetter>(impactSet.FormKey);
            });

            using var connection = await LoadAsync(plugin);
            var row = connection.QuerySingle("SELECT tag, impact_data_set FROM footsteps WHERE form_key = @key", new { key = plugin["RightFoot"] });

            Assert.Equal(("FootRight", plugin["Impacts"]), (row.tag, row.impact_data_set));
        }

        [Fact]
        public async Task ImpactDataSet_KeepsTheMaterialImpactPairsInOrder()
        {
            var plugin = new TinyPlugin();
            var dirt = plugin.AddMaterialType("Dirt");
            var stone = plugin.AddMaterialType("Stone");
            var dirtImpact = plugin.AddImpact("DirtImpact", _ => { });
            var stoneImpact = plugin.AddImpact("StoneImpact", _ => { });
            plugin.AddImpactDataSet("Impacts", set => set.Impacts.AddRange(
            [
                new ImpactData { Material = new FormLink<IMaterialTypeGetter>(stone.FormKey), Impact = new FormLink<IImpactGetter>(stoneImpact.FormKey) },
                new ImpactData { Material = new FormLink<IMaterialTypeGetter>(dirt.FormKey), Impact = new FormLink<IImpactGetter>(dirtImpact.FormKey) },
            ]));

            using var connection = await LoadAsync(plugin);
            var rows = connection.Query<(long Ord, string Material, string Impact)>("SELECT ord, material, impact FROM impact_data_set_entries WHERE set_key = @key ORDER BY ord", new { key = plugin["Impacts"] }).ToList();

            Assert.Equal([(0L, plugin["Stone"], plugin["StoneImpact"]), (1L, plugin["Dirt"], plugin["DirtImpact"])], rows);
        }

        [Fact]
        public async Task Impact_MapsSnamToTheNpcSoundAndNam1ToThePlayerSound()
        {
            var plugin = new TinyPlugin();
            var npc = plugin.AddSoundDescriptor("NpcStep", descriptor => descriptor.Data = Standard(0, 0, 0, 0, 0f));
            var player = plugin.AddSoundDescriptor("PlayerStep", descriptor => descriptor.Data = Standard(0, 0, 0, 0, 0f));
            plugin.AddImpact("Impact", impact =>
            {
                impact.Sound1 = new FormLinkNullable<ISoundGetter>(npc.FormKey);
                impact.Sound2 = new FormLinkNullable<ISoundGetter>(player.FormKey);
            });

            using var connection = await LoadAsync(plugin);
            var row = connection.QuerySingle("SELECT player_sound, npc_sound FROM impacts WHERE form_key = @key", new { key = plugin["Impact"] });

            Assert.Equal((plugin["PlayerStep"], plugin["NpcStep"]), (row.player_sound, row.npc_sound));
        }

        [Fact]
        public async Task DefaultObjectManager_StoresEverySlotWithItsTargetOrNull()
        {
            var plugin = new TinyPlugin();
            var set = plugin.AddFootstepSet("DefaultSet", _ => { });
            plugin.AddDefaultObjectManager("Manager", manager => manager.Objects =
            [
                new DefaultObjectUse { Use = new RecordType("DFTS"), Object = new FormLink<IFallout4MajorRecordGetter>(set.FormKey) },
                new DefaultObjectUse { Use = new RecordType("WWSP"), Object = new FormLink<IFallout4MajorRecordGetter>(FormKey.Null) },
            ]);

            using var connection = await LoadAsync(plugin);
            var rows = connection.Query<(string Slot, string? Target)>("SELECT slot, target FROM default_object_slots WHERE manager = @key ORDER BY slot", new { key = plugin["Manager"] }).ToList();

            Assert.Equal([("DFTS", plugin["DefaultSet"]), ("WWSP", null)], rows);
        }
    }
}
