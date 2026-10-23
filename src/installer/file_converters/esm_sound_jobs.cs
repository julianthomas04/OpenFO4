using Dapper;
using Microsoft.Data.Sqlite;
using Mutagen.Bethesda.Fallout4;
using Mutagen.Bethesda.Plugins;
using Noggog;

namespace AssetConverter
{
    internal sealed record SoundDescriptorRow(
        long DescriptorType, string? BaseDescriptor, int? FreqShiftPct, int? FreqVariancePct, int? Priority, int? DbVariance,
        int? StaticAttenuationCentidb, long? LoopingFlags);

    /// <summary>The scalar columns of one SNDR; the Standard BNAM fields are NULL for the other descriptor types.</summary>
    internal static class SoundDescriptorMapper
    {
        public const uint StandardType = 0x1EEF540A;
        public const uint CompoundType = 0x54651A43;
        public const uint AutoweaponType = 0xED157AE3;

        // Mutagen reads each BNAM percent byte as raw / 255, so the raw byte is recovered by scaling back.
        private const double PercentByteScale = 255.0;
        private const double CentidbPerDb = 100.0;

        public static SoundDescriptorRow Map(ISoundDescriptorGetter record, string subject)
        {
            long? looping = record.LoopAndRumble is { } loop
                ? loop.Unknown | ((long)(byte)loop.Loop << 8) | ((long)loop.Sidechain << 16) | ((long)loop.RumbleValues << 24)
                : null;

            return record.Data switch
            {
                ISoundDescriptorStandardDataGetter standard => new SoundDescriptorRow(
                    StandardType, null,
                    unchecked((sbyte)RawPercentByte(standard.PercentFrequencyShift)),
                    unchecked((sbyte)RawPercentByte(standard.PercentFrequencyVariance)),
                    unchecked((byte)standard.Priority),
                    unchecked((byte)standard.Variance),
                    (int)Math.Round(standard.StaticAttenuation * CentidbPerDb),
                    looping),
                ISoundDescriptorAutoweaponDataGetter autoweapon => new SoundDescriptorRow(
                    AutoweaponType, LinkKey.Text(autoweapon.BaseDescriptor), null, null, null, null, null, looping),
                ISoundDescriptorCompoundDataGetter => new SoundDescriptorRow(CompoundType, null, null, null, null, null, null, looping),
                null => throw new UnsupportedEsmDataException($"{subject}: the record has no CNAM, so its descriptor type is unknown."),
                var other => throw new UnsupportedEsmDataException($"{subject}: descriptor data of type {other.GetType().Name} has no database columns."),
            };
        }

        private static int RawPercentByte(Percent percent) => (int)Math.Round(percent.Value * PercentByteScale);
    }

    internal sealed class SoundDescriptorProcessor : BaseRecordProcessor<ISoundDescriptorGetter>
    {
        protected override async Task ProcessSpecificRecordAsync(ISoundDescriptorGetter record, string formKey, SqliteConnection conn, SqliteTransaction transaction)
        {
            string subject = $"SNDR {formKey} ({record.EditorID})";
            var row = SoundDescriptorMapper.Map(record, subject);
            await conn.ExecuteAsync(
                @"INSERT OR REPLACE INTO sound_descriptors
                    (form_key, editor_id, category, output_model, sound_files, descriptor_type, base_descriptor,
                     freq_shift_pct, freq_variance_pct, priority, db_variance, static_attenuation_centidb, looping_flags)
                  VALUES (@FormKey, @EditorId, @Category, @OutputModel, @SoundFiles, @DescriptorType, @BaseDescriptor,
                          @FreqShiftPct, @FreqVariancePct, @Priority, @DbVariance, @StaticAttenuationCentidb, @LoopingFlags)",
                new
                {
                    FormKey = formKey,
                    EditorId = record.EditorID,
                    Category = LinkKey.Text(record.Category),
                    OutputModel = LinkKey.Text(record.OutputModel),
                    SoundFiles = SafeSerialize(record.SoundFiles?.ToList()),
                    row.DescriptorType,
                    row.BaseDescriptor,
                    row.FreqShiftPct,
                    row.FreqVariancePct,
                    row.Priority,
                    row.DbVariance,
                    row.StaticAttenuationCentidb,
                    row.LoopingFlags,
                }, transaction);

            var children = (record.Descriptors ?? []).Select((link, ord) => new
            {
                Descriptor = formKey,
                Ord = ord,
                Child = link.IsNull
                    ? throw new UnsupportedEsmDataException($"{subject}: child descriptor {ord} is a null link.")
                    : link.FormKey.ToString(),
            }).ToList();
            await conn.ExecuteAsync("INSERT OR REPLACE INTO sound_descriptor_children (descriptor, ord, child) VALUES (@Descriptor, @Ord, @Child)", children, transaction);

            await ConditionTable.InsertAsync(conn, transaction, "sound_descriptor_conditions", "descriptor", formKey, record.Conditions, subject);
        }
    }

    /// <summary>The SOPM speaker matrix: three channels, each giving the percentage sent to each of the eight speakers.</summary>
    internal static class SoundOutputChannelRows
    {
        public static IReadOnlyList<(int Channel, string Speaker, int Percent)> Map(ISoundOutputChannelsGetter? channels)
        {
            if (channels is null) return [];
            var rows = new List<(int, string, int)>();
            var perChannel = new[] { channels.Channel0, channels.Channel1, channels.Channel2 };
            for (int channel = 0; channel < perChannel.Length; channel++)
            {
                var c = perChannel[channel];
                (string Speaker, int Percent)[] speakers = [("L", c.L), ("R", c.R), ("C", c.C), ("LFE", c.LFE), ("RL", c.RL), ("RR", c.RR), ("BL", c.BL), ("BR", c.BR)];
                rows.AddRange(speakers.Select(speaker => (channel, speaker.Speaker, speaker.Percent)));
            }
            return rows;
        }
    }

    internal sealed class SoundOutputModelProcessor : BaseRecordProcessor<ISoundOutputModelGetter>
    {
        private const double CentidbPerDb = 100.0;

        protected override async Task ProcessSpecificRecordAsync(ISoundOutputModelGetter record, string formKey, SqliteConnection conn, SqliteTransaction transaction)
        {
            await conn.ExecuteAsync(
                @"INSERT OR REPLACE INTO sound_output_models (form_key, editor_id, output_type, static_attenuation_db)
                  VALUES (@FormKey, @EditorId, @OutputType, @StaticAttenuationDb)",
                new
                {
                    FormKey = formKey,
                    EditorId = record.EditorID,
                    OutputType = (int?)record.Type,
                    StaticAttenuationDb = record.StaticAttenuation is { } attenuation ? Math.Round(attenuation * CentidbPerDb) / CentidbPerDb : (double?)null,
                }, transaction);

            var channels = SoundOutputChannelRows.Map(record.OutputChannels)
                .Select(row => new { Model = formKey, Ord = row.Channel, row.Speaker, row.Percent }).ToList();
            await conn.ExecuteAsync("INSERT OR REPLACE INTO sound_output_channels (model, ord, speaker, percent) VALUES (@Model, @Ord, @Speaker, @Percent)", channels, transaction);
        }
    }

    /// <summary>
    /// XCNT counts the footsteps per gait in the order walk, run, sprint, sneak, swim, but DATA stores the groups in the reverse
    /// order. Mutagen fills its five lists from the counts in forward order, so the lists name the wrong gaits; they are
    /// concatenated back into the stored array and cut by the counts again.
    /// </summary>
    internal static class FootstepSetGroups
    {
        public static readonly IReadOnlyList<string> GaitsInCountOrder = ["walk", "run", "sprint", "sneak", "swim"];

        public static IReadOnlyList<(string Gait, int Ord, string Footstep)> Split(IFootstepSetGetter record, string subject)
        {
            var lists = new[] { record.WalkFootsteps, record.RunFootsteps, record.SprintFootsteps, record.SneakFootsteps, record.SwimFootsteps };
            var stored = lists.SelectMany(list => list ?? []).Select((link, index) => link.IsNull
                ? throw new UnsupportedEsmDataException($"{subject}: stored footstep {index} is a null link.")
                : link.FormKey.ToString()).ToList();
            return Split(lists.Select(list => list?.Count ?? 0).ToList(), stored, subject);
        }

        public static IReadOnlyList<(string Gait, int Ord, string Footstep)> Split(IReadOnlyList<int> counts, IReadOnlyList<string> stored, string subject)
        {
            if (counts.Count != GaitsInCountOrder.Count)
                throw new UnsupportedEsmDataException($"{subject}: XCNT holds {counts.Count} counts, expected {GaitsInCountOrder.Count}.");
            if (counts.Any(count => count < 0) || counts.Sum() != stored.Count)
                throw new UnsupportedEsmDataException($"{subject}: XCNT counts sum to {counts.Sum()} but DATA holds {stored.Count} footsteps.");

            var rows = new List<(string, int, string)>();
            int offset = 0;
            for (int gait = GaitsInCountOrder.Count - 1; gait >= 0; gait--)
            {
                for (int ord = 0; ord < counts[gait]; ord++)
                    rows.Add((GaitsInCountOrder[gait], ord, stored[offset++]));
            }
            return rows;
        }
    }

    internal sealed class FootstepSetProcessor : BaseRecordProcessor<IFootstepSetGetter>
    {
        protected override async Task ProcessSpecificRecordAsync(IFootstepSetGetter record, string formKey, SqliteConnection conn, SqliteTransaction transaction)
        {
            await conn.ExecuteAsync("INSERT OR REPLACE INTO footstep_sets (form_key, editor_id) VALUES (@FormKey, @EditorId)",
                new { FormKey = formKey, EditorId = record.EditorID }, transaction);

            var entries = FootstepSetGroups.Split(record, $"FSTS {formKey} ({record.EditorID})")
                .Select(row => new { SetKey = formKey, row.Gait, row.Ord, row.Footstep }).ToList();
            await conn.ExecuteAsync("INSERT OR REPLACE INTO footstep_set_entries (set_key, gait, ord, footstep) VALUES (@SetKey, @Gait, @Ord, @Footstep)", entries, transaction);
        }
    }

    internal sealed class FootstepProcessor : BaseRecordProcessor<IFootstepGetter>
    {
        protected override async Task ProcessSpecificRecordAsync(IFootstepGetter record, string formKey, SqliteConnection conn, SqliteTransaction transaction)
        {
            await conn.ExecuteAsync("INSERT OR REPLACE INTO footsteps (form_key, editor_id, tag, impact_data_set) VALUES (@FormKey, @EditorId, @Tag, @ImpactDataSet)",
                new { FormKey = formKey, EditorId = record.EditorID, Tag = record.Tag, ImpactDataSet = LinkKey.Text(record.ImpactDataSet) }, transaction);
        }
    }

    internal sealed class ImpactDataSetProcessor : BaseRecordProcessor<IImpactDataSetGetter>
    {
        protected override async Task ProcessSpecificRecordAsync(IImpactDataSetGetter record, string formKey, SqliteConnection conn, SqliteTransaction transaction)
        {
            string subject = $"IPDS {formKey} ({record.EditorID})";
            await conn.ExecuteAsync("INSERT OR REPLACE INTO impact_data_sets (form_key, editor_id) VALUES (@FormKey, @EditorId)",
                new { FormKey = formKey, EditorId = record.EditorID }, transaction);

            var entries = (record.Impacts ?? []).Select((pair, ord) => new
            {
                SetKey = formKey,
                Ord = ord,
                Material = pair.Material.IsNull
                    ? throw new UnsupportedEsmDataException($"{subject}: entry {ord} has a null material.")
                    : pair.Material.FormKey.ToString(),
                Impact = pair.Impact.IsNull
                    ? throw new UnsupportedEsmDataException($"{subject}: entry {ord} has a null impact.")
                    : pair.Impact.FormKey.ToString(),
            }).ToList();
            await conn.ExecuteAsync("INSERT OR REPLACE INTO impact_data_set_entries (set_key, ord, material, impact) VALUES (@SetKey, @Ord, @Material, @Impact)", entries, transaction);
        }
    }

    /// <summary>Mutagen names the two IPCT sound links after the field order: Sound1 is SNAM (the NPC sound), Sound2 is NAM1 (the player sound).</summary>
    internal static class ImpactSounds
    {
        public static (string? Player, string? Npc) Map(IImpactGetter record) => (LinkKey.Text(record.Sound2), LinkKey.Text(record.Sound1));
    }

    internal sealed class ImpactProcessor : BaseRecordProcessor<IImpactGetter>
    {
        protected override async Task ProcessSpecificRecordAsync(IImpactGetter record, string formKey, SqliteConnection conn, SqliteTransaction transaction)
        {
            var (player, npc) = ImpactSounds.Map(record);
            await conn.ExecuteAsync("INSERT OR REPLACE INTO impacts (form_key, editor_id, player_sound, npc_sound) VALUES (@FormKey, @EditorId, @PlayerSound, @NpcSound)",
                new { FormKey = formKey, EditorId = record.EditorID, PlayerSound = player, NpcSound = npc }, transaction);
        }
    }

    internal sealed class DefaultObjectManagerProcessor : BaseRecordProcessor<IDefaultObjectManagerGetter>
    {
        protected override async Task ProcessSpecificRecordAsync(IDefaultObjectManagerGetter record, string formKey, SqliteConnection conn, SqliteTransaction transaction)
        {
            string subject = $"DOMD {formKey} ({record.EditorID})";
            await conn.ExecuteAsync("INSERT OR REPLACE INTO default_object_managers (form_key, editor_id) VALUES (@FormKey, @EditorId)",
                new { FormKey = formKey, EditorId = record.EditorID }, transaction);

            var slots = (record.Objects ?? []).Select(use => new { Manager = formKey, Slot = use.Use.Type, Target = LinkKey.Text(use.Object) }).ToList();
            var duplicate = slots.GroupBy(slot => slot.Slot, StringComparer.Ordinal).FirstOrDefault(group => group.Count() > 1);
            if (duplicate is not null)
                throw new UnsupportedEsmDataException($"{subject}: the slot {duplicate.Key} appears {duplicate.Count()} times.");
            await conn.ExecuteAsync("INSERT OR REPLACE INTO default_object_slots (manager, slot, target) VALUES (@Manager, @Slot, @Target)", slots, transaction);
        }
    }
}
