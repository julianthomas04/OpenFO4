using System.Buffers.Binary;
using AssetConverter;
using AssetConverter.Pipeline;
using Dapper;
using Microsoft.Data.Sqlite;
using Mutagen.Bethesda.Fallout4;
using Xunit;

namespace InstallerTests
{
    /// <summary>
    /// Checks the sound and footstep rows against the Fallout4.esm of the install named by <c>OPENFO4_FO4_PATH</c>, reading the plugin
    /// directly (no conversion run) and comparing Mutagen's reading with the raw subrecord bytes. Explicit: run with
    /// <c>-- xUnit.Explicit=only</c>; it never runs in the regular suite.
    /// </summary>
    public sealed class EsmSoundRealInstallTests : IDisposable
    {
        const int FootstepSetCount = 30;
        const int FootstepCount = 133;
        const int ImpactDataSetCount = 293;
        const int ImpactCount = 619;
        const string DefaultSet = "DefaultFootstepSetXXX";
        const string ImpactPlayerCompound = "FSTPlayerDirtWalkR";
        const string FirstPersonLayer = "FSTPlayerDirtWalkRLAYER1st";
        const string ThirdPersonLayer = "FSTPlayerDirtWalkRLAYER3rd";
        const int FirstPersonCondition = 391;
        const string PlayerSoundSubrecord = "NAM1";
        const string NpcSoundSubrecord = "SNAM";
        const string Plugin = "Fallout4.esm";
        static readonly string[] ReloadedTables =
        [
            "sound_descriptors", "sound_descriptor_children", "sound_descriptor_conditions", "sound_output_models", "sound_output_channels",
            "footstep_sets", "footstep_set_entries", "footsteps", "impact_data_sets", "impact_data_set_entries", "impacts",
            "default_object_managers", "default_object_slots", "music_tracks", "music_track_conditions",
        ];

        readonly TempDirectory _directory = new();
        readonly string _esmPath;
        readonly IFallout4ModDisposableGetter _file;

        public EsmSoundRealInstallTests()
        {
            string? game = Environment.GetEnvironmentVariable("OPENFO4_FO4_PATH");
            Assert.SkipWhen(string.IsNullOrEmpty(game), "OPENFO4_FO4_PATH names no Fallout 4 install.");
            _esmPath = Path.Combine(game!, "Data", "Fallout4.esm");
            _file = Fallout4Mod.CreateFromBinaryOverlay(_esmPath, Fallout4Release.Fallout4);
        }

        public void Dispose()
        {
            _file.Dispose();
            SqliteConnection.ClearAllPools();
            _directory.Dispose();
        }

        sealed record RawRecord(IReadOnlyDictionary<string, List<byte[]>> Subrecords)
        {
            public byte[]? First(string signature) => Subrecords.TryGetValue(signature, out var bodies) ? bodies[0] : null;
        }

        Dictionary<string, RawRecord> ScanRaw(string recordType)
        {
            var records = new Dictionary<string, RawRecord>();
            RawEsmScanner.Scan(_esmPath, nameof(EsmSoundRealInstallTests), new HashSet<string> { recordType }, (_, data) =>
            {
                var bySignature = new Dictionary<string, List<byte[]>>();
                foreach (var (signature, body) in RawEsmScanner.Subrecords(data))
                {
                    if (!bySignature.TryGetValue(signature, out var bodies)) bySignature[signature] = bodies = [];
                    bodies.Add(body);
                }
                return new RawRecord(bySignature);
            }, records);
            return records;
        }

        static string FormKeyOf(byte[] body, int offset = 0) =>
            $"{BinaryPrimitives.ReadUInt32LittleEndian(body.AsSpan(offset)) & 0xFFFFFF:X6}:{Plugin}";

        [Fact(Explicit = true)]
        public async Task TheTableReloadProcessesEveryRecordOfTheSoundAndFootstepTypes()
        {
            string dbPath = Path.Combine(_directory.Path, "data.db");
            using (var manager = new DatabaseManager(dbPath))
                await manager.InitializeAsync(bulkLoad: false);

            using var connection = new SqliteConnection(new SqliteConnectionStringBuilder { DataSource = dbPath, Pooling = false }.ToString());
            connection.Open();
            var loader = new ESMLoader(dbPath) { Memory = MemoryBudget.Unlimited() };
            var context = new ImportContext(_file, null);
            await connection.ExecuteAsync("PRAGMA foreign_keys = OFF");
            using (var transaction = connection.BeginTransaction())
            {
                foreach (var job in TableReloadPlan.Create(ReloadedTables).Jobs)
                    await job.Run(loader, context, connection, transaction);
                transaction.Commit();
            }

            long Count(string table) => connection.ExecuteScalar<long>($"SELECT COUNT(*) FROM {table}");
            Assert.Equal(_file.SoundDescriptors.Count, Count("sound_descriptors"));
            Assert.Equal(_file.SoundDescriptors.Sum(descriptor => (long)descriptor.Descriptors.Count), Count("sound_descriptor_children"));
            Assert.Equal(_file.SoundDescriptors.Sum(descriptor => (long)(descriptor.Conditions?.Count ?? 0)), Count("sound_descriptor_conditions"));
            Assert.Equal(_file.SoundOutputModels.Count, Count("sound_output_models"));
            Assert.Equal(FootstepSetCount, Count("footstep_sets"));
            Assert.Equal(FootstepCount, Count("footsteps"));
            Assert.Equal(ImpactDataSetCount, Count("impact_data_sets"));
            Assert.Equal(_file.ImpactDataSets.Sum(set => (long)set.Impacts.Count), Count("impact_data_set_entries"));
            Assert.Equal(ImpactCount, Count("impacts"));
            Assert.Equal(_file.DefaultObjectManagers.Sum(manager => (long)(manager.Objects?.Count ?? 0)), Count("default_object_slots"));
            Assert.Equal(_file.MusicTracks.Sum(track => (long)(track.Conditions?.Count ?? 0)), Count("music_track_conditions"));
        }

        [Fact(Explicit = true)]
        public void TheBaseGameHasTheDocumentedRecordCounts()
        {
            Assert.Equal(FootstepSetCount, _file.FootstepSets.Count);
            Assert.Equal(FootstepCount, _file.Footsteps.Count);
            Assert.Equal(ImpactDataSetCount, _file.ImpactDataSets.Count);
            Assert.Equal(ImpactCount, _file.Impacts.Count);
        }

        [Fact(Explicit = true)]
        public void TheDefaultFootstepSetHasTheDocumentedGaitGroups()
        {
            var set = _file.FootstepSets.Single(candidate => candidate.EditorID == DefaultSet);
            var footsteps = _file.Footsteps.ToDictionary(footstep => footstep.FormKey.ToString());

            var byGait = FootstepSetGroups.Split(set, DefaultSet)
                .GroupBy(row => row.Gait)
                .ToDictionary(group => group.Key, group => group.OrderBy(row => row.Ord).Select(row => footsteps[row.Footstep]).ToList());

            Assert.Equal(6, byGait["walk"].Count);
            Assert.Equal(6, byGait["run"].Count);
            Assert.Equal(4, byGait["sprint"].Count);
            Assert.Equal(4, byGait["sneak"].Count);
            Assert.Equal(2, byGait["swim"].Count);
            Assert.Contains(byGait["walk"], footstep => footstep.Tag == "FootRight" && footstep.EditorID == "DefaultFootWalkRFootstep");
            Assert.Contains(byGait["run"], footstep => footstep.Tag == "FootRight" && footstep.EditorID == "DefaultFootRunRFootstep");
            Assert.Contains(byGait["sprint"], footstep => footstep.Tag == "FootSprintRight" && footstep.EditorID == "DefaultFootSprintRFootstep");
            Assert.Contains(byGait["sneak"], footstep => footstep.Tag == "FootRight" && footstep.EditorID == "DefaultFootSneakRFootstep");
            Assert.Equal(["JumpDown", "JumpUp"], byGait["swim"].Select(footstep => footstep.Tag).Order().ToList());
        }

        [Fact(Explicit = true)]
        public void EveryFootstepSetCountsSumToItsStoredArrayAndSplits()
        {
            var raw = ScanRaw("FSTS");

            Assert.Equal(FootstepSetCount, raw.Count);
            foreach (var set in _file.FootstepSets)
            {
                var record = raw[set.FormKey.ToString()];
                int countSum = Enumerable.Range(0, record.First("XCNT")!.Length / sizeof(uint))
                    .Sum(index => (int)BinaryPrimitives.ReadUInt32LittleEndian(record.First("XCNT")!.AsSpan(index * sizeof(uint))));

                Assert.Equal(countSum, record.First("DATA")!.Length / sizeof(uint));
                Assert.Equal(countSum, FootstepSetGroups.Split(set, set.EditorID ?? "").Count);
            }
        }

        [Fact(Explicit = true)]
        public void EveryImpactDataSetHasNoNullLinks()
        {
            foreach (var set in _file.ImpactDataSets)
                Assert.All(set.Impacts, pair => Assert.False(pair.Material.IsNull || pair.Impact.IsNull, set.EditorID));
        }

        [Fact(Explicit = true)]
        public void ImpactSoundsMatchTheRawNam1AndSnamSubrecords()
        {
            var raw = ScanRaw("IPCT");

            Assert.Equal(ImpactCount, raw.Count);
            foreach (var impact in _file.Impacts)
            {
                var record = raw[impact.FormKey.ToString()];
                var (player, npc) = ImpactSounds.Map(impact);

                Assert.Equal(record.First(PlayerSoundSubrecord) is { } nam1 ? FormKeyOf(nam1) : null, player);
                Assert.Equal(record.First(NpcSoundSubrecord) is { } snam ? FormKeyOf(snam) : null, npc);
            }
        }

        [Fact(Explicit = true)]
        public void TheDirtWalkImpactPlaysTheCompoundWhoseFirstChildIsTheFirstPersonLayer()
        {
            var impact = _file.Impacts.Single(candidate => candidate.EditorID == "FSTDirtWalkRImpact");
            var descriptors = _file.SoundDescriptors.ToDictionary(descriptor => descriptor.FormKey.ToString());

            var compound = descriptors[ImpactSounds.Map(impact).Player!];
            var children = compound.Descriptors.Select(link => descriptors[link.FormKey.ToString()]).ToList();

            Assert.Equal(ImpactPlayerCompound, compound.EditorID);
            Assert.Equal([FirstPersonLayer, ThirdPersonLayer], children.Select(child => child.EditorID).ToList());
            var condition = ConditionMapper.Map(children[0].Conditions.Single(), "first person layer");
            Assert.Equal((FirstPersonCondition, 0, 1.0), (condition.Function, condition.CompareOperator, condition.CompareValue));
        }

        [Fact(Explicit = true)]
        public void TheDefaultFootstepSetIsTheDftsDefaultObject()
        {
            var set = _file.FootstepSets.Single(candidate => candidate.EditorID == DefaultSet);
            var slot = _file.DefaultObjectManagers.Single().Objects!.Single(use => use.Use.Type == "DFTS");

            Assert.Equal(set.FormKey, slot.Object.FormKey);
        }

        [Fact(Explicit = true)]
        public void EverySoundDescriptorMatchesItsRawCnamBnamAndLnam()
        {
            var raw = ScanRaw("SNDR");

            Assert.Equal(_file.SoundDescriptors.Count, raw.Count);
            foreach (var descriptor in _file.SoundDescriptors)
            {
                string subject = descriptor.EditorID ?? descriptor.FormKey.ToString();
                var record = raw[descriptor.FormKey.ToString()];
                var row = SoundDescriptorMapper.Map(descriptor, subject);
                var bnam = record.First("BNAM")!;

                Assert.Equal(BinaryPrimitives.ReadUInt32LittleEndian(record.First("CNAM")!), (uint)row.DescriptorType);
                Assert.Equal(record.First("LNAM") is { } lnam ? BinaryPrimitives.ReadUInt32LittleEndian(lnam) : (long?)null, row.LoopingFlags);
                if (row.DescriptorType == SoundDescriptorMapper.StandardType)
                {
                    Assert.Equal(
                        $"{subject}: {(sbyte)bnam[0]} {(sbyte)bnam[1]} {bnam[2]} {bnam[3]} {BinaryPrimitives.ReadUInt16LittleEndian(bnam.AsSpan(4))}",
                        $"{subject}: {row.FreqShiftPct} {row.FreqVariancePct} {row.Priority} {row.DbVariance} {row.StaticAttenuationCentidb}");
                }
                else if (row.DescriptorType == SoundDescriptorMapper.AutoweaponType)
                {
                    Assert.Equal(FormKeyOf(bnam), row.BaseDescriptor);
                }
            }
        }

        [Fact(Explicit = true)]
        public void TheFirstPersonOutputModelHasTheDocumentedMatrix()
        {
            var model = _file.SoundOutputModels.Single(candidate => candidate.EditorID == "SOMMono01500Player1st");

            var rows = SoundOutputChannelRows.Map(model.OutputChannels);

            Assert.Equal(1.76f, model.StaticAttenuation);
            Assert.Equal(75, rows.Single(row => row.Channel == 0 && row.Speaker == "L").Percent);
            Assert.Equal(75, rows.Single(row => row.Channel == 0 && row.Speaker == "R").Percent);
            Assert.Equal(100, rows.Single(row => row.Channel == 1 && row.Speaker == "L").Percent);
        }

        [Fact(Explicit = true)]
        public void EveryOutputModelStaysWithinTheRawOnamMatrix()
        {
            var raw = ScanRaw("SOPM");

            foreach (var model in _file.SoundOutputModels)
            {
                var onam = raw[model.FormKey.ToString()].First("ONAM");
                var rows = SoundOutputChannelRows.Map(model.OutputChannels);

                Assert.Equal(onam is null ? 0 : onam.Length, rows.Count);
                if (onam is not null)
                    Assert.Equal(onam.Select(percent => (int)percent).ToList(), rows.OrderBy(row => row.Channel).Select(row => row.Percent).ToList());
            }
        }
    }
}
