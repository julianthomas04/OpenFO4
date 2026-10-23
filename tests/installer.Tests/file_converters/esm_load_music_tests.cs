using System.Buffers.Binary;
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
    // The ambient-music tables. A real binary plugin built from Mutagen's in-memory
    // records is loaded through the real loader; the values mirror the base game's records.
    public sealed class EsmLoadMusicTests : IDisposable
    {
        const float ExplorePriority = 90;
        const float ExploreFadeSeconds = 12;
        const float DuckingDecibel = 12.5f;
        const int DuckingCentidb = 1250;
        const string AnamWithLowercasePrefix = @"data\Music\Special\MUS_MainTheme.wav";
        const string AnamWithLeadingSeparator = @"\Data\Music\Special\MUS_MainTheme.wav";
        const string MainThemeKey = "music/special/mus_maintheme.flac";
        const long GetCurrentTime = 18;
        const long GetGlobalValue = 74;
        const long GetStageDone = 59;

        readonly TempDirectory _directory = new();
        readonly string _dbPath;

        public EsmLoadMusicTests() => _dbPath = Path.Combine(_directory.Path, "data.db");

        public void Dispose()
        {
            SqliteConnection.ClearAllPools();
            _directory.Dispose();
        }

        SqliteConnection Open()
        {
            var connection = new SqliteConnection(new SqliteConnectionStringBuilder { DataSource = _dbPath, Pooling = false }.ToString());
            connection.Open();
            return connection;
        }

        async Task<SqliteConnection> LoadAsync(TinyPlugin plugin)
        {
            string esmPath = plugin.Write(_directory.Path);
            await new ESMLoader(_dbPath) { Memory = MemoryBudget.Unlimited() }.LoadESMAsync(esmPath, ct: TestContext.Current.CancellationToken);
            return Open();
        }

        static FormLink<IMusicTrackGetter> Link(MusicTrack track) => new FormLink<IMusicTrackGetter>(track.FormKey);

        static FormLink<IMusicTrackGetter> Separator() => new FormLink<IMusicTrackGetter>(FormKey.Null);

        static FunctionConditionData Function(Condition.Function function, Action<FunctionConditionData>? configure = null)
        {
            var data = new FunctionConditionData { Function = function };
            configure?.Invoke(data);
            return data;
        }

        static ConditionFloat Literal(Condition.Function function, CompareOperator op, float value, Condition.Flag flags = 0) =>
            new() { Data = Function(function), CompareOperator = op, ComparisonValue = value, Flags = flags };

        [Fact]
        public async Task MusicType_DecodesFlagsPriorityDuckingAndFade()
        {
            var plugin = new TinyPlugin();
            plugin.AddMusicType("MUSzExplore", type =>
            {
                type.Flags = MusicType.Flag.CycleTracks | MusicType.Flag.DucksCurrentTrack;
                type.Data = new MusicTypeData { Priority = (ushort)ExplorePriority, DuckingDecibel = DuckingDecibel };
                type.FadeDuration = ExploreFadeSeconds;
            });

            using var connection = await LoadAsync(plugin);
            var row = connection.QuerySingle("SELECT flags, priority, ducking_centidb, fade_seconds FROM music_types WHERE form_key = @key", new { key = plugin["MUSzExplore"] });

            Assert.Equal(4 | 32, (int)row.flags);
            Assert.Equal((long)ExplorePriority, row.priority);
            Assert.Equal((long)DuckingCentidb, row.ducking_centidb);
            Assert.Equal((double)ExploreFadeSeconds, row.fade_seconds);
        }

        [Fact]
        public async Task MusicType_KeepsTheTrackListInOrderWithDuplicates()
        {
            var plugin = new TinyPlugin();
            var first = plugin.AddMusicTrack("TrackA", track => track.Type = MusicTrack.TypeEnum.SilentTrack);
            var second = plugin.AddMusicTrack("TrackB", track => track.Type = MusicTrack.TypeEnum.SilentTrack);
            plugin.AddMusicType("MUSzExplore", type =>
            {
                type.Data = new MusicTypeData();
                type.Tracks = [Link(first), Link(second), Link(first)];
            });

            using var connection = await LoadAsync(plugin);
            var rows = connection.Query<(long Ord, string Track)>("SELECT ord, track FROM music_type_tracks WHERE music_type = @key ORDER BY ord", new { key = plugin["MUSzExplore"] }).ToList();

            Assert.Equal([(0L, plugin["TrackA"]), (1L, plugin["TrackB"]), (2L, plugin["TrackA"])], rows);
        }

        [Theory]
        [InlineData(AnamWithLowercasePrefix)]
        [InlineData(AnamWithLeadingSeparator)]
        public async Task SingleTrack_StoresTheAudioKeyTheAudioLaneWritesForTheSameFile(string anam)
        {
            var plugin = new TinyPlugin();
            plugin.AddMusicTrack("MUSMainTheme", track =>
            {
                track.Type = MusicTrack.TypeEnum.SingleTrack;
                track.Duration = 93.5f;
                track.FadeOut = 4f;
                track.TrackFilename = anam;
            });

            using var connection = await LoadAsync(plugin);
            var row = connection.QuerySingle("SELECT track_type, duration, fade_out, file_key, anam_path FROM music_tracks WHERE form_key = @key", new { key = plugin["MUSMainTheme"] });

            Assert.Equal("Single", row.track_type);
            Assert.Equal(93.5, row.duration);
            Assert.Equal(4.0, row.fade_out);
            Assert.Equal(MainThemeKey, row.file_key);
            Assert.Equal(anam, row.anam_path);
        }

        [Fact]
        public async Task SilentTrack_NamesNoFile()
        {
            var plugin = new TinyPlugin();
            plugin.AddMusicTrack("MUSSilence", track => track.Type = MusicTrack.TypeEnum.SilentTrack);

            using var connection = await LoadAsync(plugin);
            var row = connection.QuerySingle("SELECT track_type, file_key, anam_path FROM music_tracks WHERE form_key = @key", new { key = plugin["MUSSilence"] });

            Assert.Equal("Silent", row.track_type);
            Assert.Null(row.file_key);
            Assert.Null(row.anam_path);
        }

        [Fact]
        public async Task Palette_SplitsItsSubTrackListIntoLayersAtTheNullEntries()
        {
            var plugin = new TinyPlugin();
            var a = plugin.AddMusicTrack("PartA", track => track.Type = MusicTrack.TypeEnum.SilentTrack);
            var b = plugin.AddMusicTrack("PartB", track => track.Type = MusicTrack.TypeEnum.SilentTrack);
            var c = plugin.AddMusicTrack("PartC", track => track.Type = MusicTrack.TypeEnum.SilentTrack);
            plugin.AddMusicTrack("Palette", track =>
            {
                track.Type = MusicTrack.TypeEnum.Palette;
                track.Tracks = [Link(a), Link(b), Separator(), Link(c), Link(a)];
            });

            using var connection = await LoadAsync(plugin);
            var rows = connection.Query<(long Layer, long Ord, string SubTrack)>("SELECT layer, ord, sub_track FROM music_track_layers WHERE track = @key ORDER BY layer, ord", new { key = plugin["Palette"] }).ToList();

            Assert.Equal([(0L, 0L, plugin["PartA"]), (0L, 1L, plugin["PartB"]), (1L, 0L, plugin["PartC"]), (1L, 1L, plugin["PartA"])], rows);
        }

        [Fact]
        public async Task TrackConditions_KeepTheOrFlagTheGlobalCompareAndTheParameters()
        {
            var plugin = new TinyPlugin();
            var quest = plugin.AddKeyword("StandInQuest");
            var timeScale = plugin.AddGlobal("TimeScale", mod => new GlobalShort(mod) { Data = 20 });
            plugin.AddMusicTrack("DayTrack", track =>
            {
                track.Type = MusicTrack.TypeEnum.SilentTrack;
                track.Conditions =
                [
                    Literal(Condition.Function.GetCurrentTime, CompareOperator.GreaterThanOrEqualTo, 6f, Condition.Flag.OR),
                    Literal(Condition.Function.GetCurrentTime, CompareOperator.LessThan, 18f),
                    new ConditionGlobal
                    {
                        Data = Function(Condition.Function.GetGlobalValue, data => data.ParameterOneRecord = new FormLink<IFallout4MajorRecordGetter>(timeScale.FormKey)),
                        CompareOperator = CompareOperator.NotEqualTo,
                        ComparisonValue = new FormLink<IGlobalGetter>(timeScale.FormKey),
                    },
                    new ConditionFloat
                    {
                        Data = Function(Condition.Function.GetStageDone, data =>
                        {
                            data.ParameterOneRecord = new FormLink<IFallout4MajorRecordGetter>(quest.FormKey);
                            data.ParameterTwoNumber = 10;
                        }),
                        CompareOperator = CompareOperator.EqualTo,
                        ComparisonValue = 1f,
                    },
                ];
            });

            using var connection = await LoadAsync(plugin);
            var rows = connection.Query("SELECT * FROM music_track_conditions WHERE track = @key ORDER BY ord", new { key = plugin["DayTrack"] }).ToList();

            Assert.Equal(4, rows.Count);
            Assert.Equal((GetCurrentTime, 3L, 1L, 6.0, null), (rows[0].function, rows[0].compare_op, rows[0].or_flag, rows[0].compare_value, rows[0].compare_global));
            Assert.Equal((GetCurrentTime, 4L, 0L, 18.0, null), (rows[1].function, rows[1].compare_op, rows[1].or_flag, rows[1].compare_value, rows[1].compare_global));
            Assert.Equal((GetGlobalValue, 1L, null, plugin["TimeScale"], plugin["TimeScale"]), (rows[2].function, rows[2].compare_op, rows[2].compare_value, rows[2].compare_global, rows[2].param1_record));
            Assert.Equal((GetStageDone, 0L, 1.0, plugin["StandInQuest"], 10L), (rows[3].function, rows[3].compare_op, rows[3].compare_value, rows[3].param1_record, rows[3].param2_num));
            Assert.All(rows, row => Assert.Equal(0L, row.run_on));
        }

        [Fact]
        public void ConditionMapper_KeepsTheTypeBitsOtherThanOrInFlags()
        {
            var condition = Literal(Condition.Function.GetCurrentTime, CompareOperator.EqualTo, 1f,
                Condition.Flag.OR | Condition.Flag.UsePackData | Condition.Flag.SwapSubjectAndTarget | Condition.Flag.ParametersUseAliases);

            var row = ConditionMapper.Map(condition, "subject");

            Assert.True(row.OrFlag);
            Assert.Equal(2 | 8 | 16, row.Flags);
            Assert.Equal(0, ConditionMapper.Map(Literal(Condition.Function.GetCurrentTime, CompareOperator.EqualTo, 1f, Condition.Flag.OR), "subject").Flags);
        }

        [Fact]
        public void ConditionMapper_RejectsShapesTheTableCannotHold()
        {
            var eventData = new ConditionFloat { Data = new GetEventData(), ComparisonValue = 1f };
            var missingGlobal = new ConditionGlobal { Data = Function(Condition.Function.GetCurrentTime) };

            Assert.Throws<UnsupportedEsmDataException>(() => ConditionMapper.Map(eventData, "subject"));
            Assert.Throws<UnsupportedEsmDataException>(() => ConditionMapper.Map(missingGlobal, "subject"));
        }

        [Fact]
        public void MusicLayers_RejectEmptyLayers()
        {
            var track = new FormLink<IMusicTrackGetter>(new FormKey(ModKey.FromNameAndExtension("A.esm"), 0x800));
            var separator = Separator();

            Assert.Throws<UnsupportedEsmDataException>(() => MusicLayers.Split([separator, track], "subject"));
            Assert.Throws<UnsupportedEsmDataException>(() => MusicLayers.Split([track, separator], "subject"));
            Assert.Throws<UnsupportedEsmDataException>(() => MusicLayers.Split([track, separator, separator, track], "subject"));
        }

        [Fact]
        public async Task CellPlaces_RecordsMusicAndLocationAndSkipsCellsWithNeither()
        {
            var plugin = new TinyPlugin();
            var explore = plugin.AddMusicType("MUSzExplore", type => type.Data = new MusicTypeData());
            var location = plugin.AddLocation("FixtureLocation", _ => { });
            plugin.AddWorldspace("FixtureWorld");
            plugin.Exterior.Music.SetTo(explore.FormKey);
            plugin.Exterior.Location.SetTo(location.FormKey);
            plugin.Interior.Location.SetTo(location.FormKey);

            using var connection = await LoadAsync(plugin);
            var rows = connection.Query<(string Cell, string? MusicType, string? Location)>("SELECT cell, music_type, location FROM cell_places").ToDictionary(row => row.Cell);

            Assert.Equal(2, rows.Count);
            Assert.Equal((plugin["MUSzExplore"], plugin["FixtureLocation"]), (rows[plugin[TinyPlugin.ExteriorCellEditorId]].MusicType, rows[plugin[TinyPlugin.ExteriorCellEditorId]].Location));
            Assert.Equal((null, plugin["FixtureLocation"]), (rows[plugin[TinyPlugin.InteriorCellEditorId]].MusicType, rows[plugin[TinyPlugin.InteriorCellEditorId]].Location));
        }

        [Fact]
        public async Task SoundCategories_StoreTheRealStaticVolumeAndDefaultMenuValue()
        {
            var plugin = new TinyPlugin();
            var master = plugin.AddSoundCategory("MasterCategory", category => category.StaticVolumeMultiplier = 0.85f);
            plugin.AddSoundCategory("MusicCategory", category =>
            {
                category.Parent.SetTo(master.FormKey);
                category.StaticVolumeMultiplier = 0.82f;
                category.DefaultMenuVolume = 0.65f;
            });
            plugin.AddSoundCategory("BareCategory", _ => { });

            using var connection = await LoadAsync(plugin);
            var rows = connection.Query<(string EditorId, string? Parent, double? Static, double? Menu)>(
                "SELECT editor_id, parent, static_attenuation, default_menu_volume FROM sound_categories").ToDictionary(row => row.EditorId);

            Assert.Equal((null, 0.85, null), (rows["MasterCategory"].Parent, Quantized(rows["MasterCategory"].Static), rows["MasterCategory"].Menu));
            Assert.Equal((plugin["MasterCategory"], 0.82, 0.65), (rows["MusicCategory"].Parent, Quantized(rows["MusicCategory"].Static), Quantized(rows["MusicCategory"].Menu)));
            Assert.Null(rows["BareCategory"].Static);
            Assert.Null(rows["BareCategory"].Menu);
        }

        [Fact]
        public async Task LocationAndWorldspaceMusic_RecordOnlyTheOnesThatNameMusic()
        {
            var plugin = new TinyPlugin();
            var explore = plugin.AddMusicType("MUSzExplore", type => type.Data = new MusicTypeData());
            var withMusic = plugin.AddLocation("MusicLocation", location => location.Music.SetTo(explore.FormKey));
            plugin.AddLocation("SilentLocation", _ => { });
            plugin.AddWorldspace("FixtureWorld").Music.SetTo(explore.FormKey);

            using var connection = await LoadAsync(plugin);

            Assert.Equal([(withMusic.FormKey.ToString(), plugin["MUSzExplore"])], connection.Query<(string, string)>("SELECT location, music_type FROM location_music").ToList());
            Assert.Equal([(plugin["FixtureWorld"], plugin["MUSzExplore"])], connection.Query<(string, string)>("SELECT worldspace, music_type FROM worldspace_music").ToList());
        }

        [Fact]
        public async Task RegionMusic_StoresPriorityAndTheOverrideFlagForRegionsWithMusic()
        {
            var plugin = new TinyPlugin();
            var downtown = plugin.AddMusicType("MUSDowntown", type => type.Data = new MusicTypeData());
            plugin.AddRegion("AudioNoMusic", region => region.Sounds = new RegionSounds { Priority = 51 });
            plugin.AddRegion("AudioExtDowntownAUDIO", region => region.Sounds = new RegionSounds { Music = new FormLinkNullable<IMusicTypeGetter>(downtown.FormKey), Priority = 55, Flags = RegionData.RegionDataFlag.Override });
            plugin.AddRegion("AudioExtSanctuaryHills", region => region.Sounds = new RegionSounds { Music = new FormLinkNullable<IMusicTypeGetter>(downtown.FormKey), Priority = 55 });

            using var connection = await LoadAsync(plugin);
            var rows = connection.Query<(string Region, string MusicType, long Priority, long Override, long RecordOrder)>("SELECT region, music_type, priority, override, record_order FROM region_music").ToDictionary(row => row.Region);

            Assert.Equal(2, rows.Count);
            Assert.Equal((plugin["MUSDowntown"], 55L, 1L), (rows[plugin["AudioExtDowntownAUDIO"]].MusicType, rows[plugin["AudioExtDowntownAUDIO"]].Priority, rows[plugin["AudioExtDowntownAUDIO"]].Override));
            Assert.Equal((plugin["MUSDowntown"], 55L, 0L), (rows[plugin["AudioExtSanctuaryHills"]].MusicType, rows[plugin["AudioExtSanctuaryHills"]].Priority, rows[plugin["AudioExtSanctuaryHills"]].Override));
        }

        [Fact]
        public async Task RegionMusic_RecordOrderIsThePositionAmongAllRegionRecords()
        {
            var plugin = new TinyPlugin();
            var downtown = plugin.AddMusicType("MUSDowntown", type => type.Data = new MusicTypeData());
            Action<Mutagen.Bethesda.Fallout4.Region> withMusic = region => region.Sounds = new RegionSounds { Music = new FormLinkNullable<IMusicTypeGetter>(downtown.FormKey), Priority = 55 };
            plugin.AddRegion("AudioFirstNoMusic", _ => { });
            plugin.AddRegion("AudioSecond", withMusic);
            plugin.AddRegion("AudioThirdNoMusic", _ => { });
            plugin.AddRegion("AudioFourth", withMusic);

            using var connection = await LoadAsync(plugin);
            var order = connection.Query<(string Region, long RecordOrder)>("SELECT region, record_order FROM region_music").ToDictionary(row => row.Region, row => row.RecordOrder);

            Assert.Equal(1L, order[plugin["AudioSecond"]]);
            Assert.Equal(3L, order[plugin["AudioFourth"]]);
        }

        [Fact]
        public async Task MusicType_WithoutPriorityBlockIsAnErrorNamingTheRecord()
        {
            var plugin = new TinyPlugin();
            plugin.AddMusicType("MUSNoPnam", _ => { });

            var error = await Assert.ThrowsAnyAsync<Exception>(() => LoadAsync(plugin));

            Assert.Contains("MUSNoPnam", Flatten(error), StringComparison.Ordinal);
        }

        [Fact]
        public async Task SingleTrack_WithoutAnamIsAnErrorNamingTheRecord()
        {
            var plugin = new TinyPlugin();
            plugin.AddMusicTrack("MUSNoAnam", track => track.Type = MusicTrack.TypeEnum.SingleTrack);

            var error = await Assert.ThrowsAnyAsync<Exception>(() => LoadAsync(plugin));

            Assert.Contains("MUSNoAnam", Flatten(error), StringComparison.Ordinal);
        }

        static string Flatten(Exception error) => error is AggregateException aggregate
            ? string.Join(" | ", aggregate.Flatten().InnerExceptions.Select(Flatten))
            : error.Message + (error.InnerException is null ? "" : " | " + Flatten(error.InnerException));

        [Fact]
        public async Task RegionAreas_StoreOneRowPerAreaWithLittleEndianFloatPairs()
        {
            var plugin = new TinyPlugin();
            P2Float[] outline = [new(-4096.5f, 8192f), new(0f, 1.25f), new(300000f, -300000f)];
            plugin.AddRegion("AudioMarsh", region =>
            {
                region.RegionAreas.Add(new RegionArea { EdgeFallOff = 3, RegionPointListData = [.. outline] });
                region.RegionAreas.Add(new RegionArea { EdgeFallOff = 0, RegionPointListData = [new P2Float(1f, 2f)] });
            });

            using var connection = await LoadAsync(plugin);
            var rows = connection.Query<(long Area, long EdgeFalloff, byte[] Points)>("SELECT area, edge_falloff, points FROM region_areas WHERE region = @key ORDER BY area", new { key = plugin["AudioMarsh"] }).ToList();

            Assert.Equal([0L, 1L], rows.Select(row => row.Area));
            Assert.Equal([3L, 0L], rows.Select(row => row.EdgeFalloff));
            Assert.Equal(outline, DecodePoints(rows[0].Points));
            Assert.Equal([new P2Float(1f, 2f)], DecodePoints(rows[1].Points));
        }

        [Fact]
        public async Task Globals_StoreTheRealValueAndTypeOfEachVariant()
        {
            var plugin = new TinyPlugin();
            plugin.AddGlobal("GameHour", mod => new GlobalFloat(mod) { Data = 9.78f });
            plugin.AddGlobal("TimeScale", mod => new GlobalShort(mod) { Data = 20 });
            plugin.AddGlobal("Population", mod => new GlobalInt(mod) { Data = 123456 });
            plugin.AddGlobal("Unset", mod => new GlobalFloat(mod));

            using var connection = await LoadAsync(plugin);
            var rows = connection.Query<(string EditorId, string ValueType, double? Value)>("SELECT editor_id, value_type, value FROM globals").ToDictionary(row => row.EditorId);

            Assert.Equal("float", rows["GameHour"].ValueType);
            Assert.Equal(9.78f, (float)rows["GameHour"].Value!.Value);
            Assert.Equal(("short", 20.0), (rows["TimeScale"].ValueType, rows["TimeScale"].Value));
            Assert.Equal(("int", 123456.0), (rows["Population"].ValueType, rows["Population"].Value));
            Assert.Null(rows["Unset"].Value);
        }

        // Mutagen's binary overlay cannot read a bool global back (it decodes the one byte as a float), and the base game has none,
        // so the typed read is checked on in-memory records.
        [Theory]
        [InlineData(true, 1.0)]
        [InlineData(false, 0.0)]
        public void Globals_ReadABoolAsOneOrZero(bool flag, double expected)
        {
            var global = new GlobalBool(new Fallout4Mod(ModKey.FromNameAndExtension("Fixture.esm"), Fallout4Release.Fallout4)) { Data = flag };

            Assert.Equal(("bool", (double?)expected), GlobalsProcessor.Read(global, "GLOB fixture"));
        }

        [Fact]
        public async Task ReloadingTheMusicTables_RefillsThemWithoutTouchingTheCellsJob()
        {
            var plugin = new TinyPlugin();
            var explore = plugin.AddMusicType("MUSzExplore", type => type.Data = new MusicTypeData { Priority = 90 });
            plugin.AddWorldspace("FixtureWorld");
            plugin.Exterior.Music.SetTo(explore.FormKey);
            plugin.AddGlobal("TimeScale", mod => new GlobalShort(mod) { Data = 20 });
            plugin.AddSoundCategory("MusicCategory", category => category.StaticVolumeMultiplier = 0.82f);
            string esmPath = Path.Combine(_directory.Path, TinyPlugin.PluginFileName);
            using (await LoadAsync(plugin)) { }
            using (var connection = Open())
                connection.Execute("DELETE FROM cell_places; DELETE FROM sound_categories; DELETE FROM music_types; DELETE FROM globals");

            var plan = TableReloadPlan.Create(MusicTables);
            await new ESMLoader(_dbPath) { Memory = MemoryBudget.Unlimited() }.ReloadTablesAsync(esmPath, plan, TestContext.Current.CancellationToken);

            Assert.Equal(MusicTables.Order(StringComparer.Ordinal), plan.Tables);
            Assert.False(plan.NeedsRawScans);
            using var reloaded = Open();
            Assert.Equal(1, reloaded.ExecuteScalar<long>("SELECT COUNT(*) FROM music_types WHERE priority = 90"));
            Assert.Equal(1, reloaded.ExecuteScalar<long>("SELECT COUNT(*) FROM cell_places"));
            Assert.Equal(0.82, Quantized(reloaded.ExecuteScalar<double>("SELECT static_attenuation FROM sound_categories")));
            Assert.Equal(20.0, reloaded.ExecuteScalar<double>("SELECT value FROM globals WHERE editor_id = 'TimeScale'"));
        }

        static readonly string[] MusicTables =
        [
            "music_types", "music_type_tracks", "music_tracks", "music_track_layers", "music_track_conditions",
            "cell_places", "location_music", "worldspace_music", "region_music", "region_areas", "globals", "sound_categories",
        ];

        // The record stores the multiplier as a u16 scaled by 65535, so a written value comes back to within one step.
        static double? Quantized(double? value) => value is null ? null : Math.Round(value.Value, 2);

        static List<P2Float> DecodePoints(byte[] blob)
        {
            Assert.Equal(0, blob.Length % 8);
            return Enumerable.Range(0, blob.Length / 8)
                .Select(i => new P2Float(BinaryPrimitives.ReadSingleLittleEndian(blob.AsSpan(i * 8)), BinaryPrimitives.ReadSingleLittleEndian(blob.AsSpan(i * 8 + 4))))
                .ToList();
        }
    }
}
