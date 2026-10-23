using System.Buffers.Binary;
using AssetConverter.Audio;
using Dapper;
using Microsoft.Data.Sqlite;
using Mutagen.Bethesda.Fallout4;
using Mutagen.Bethesda.Plugins;
using Noggog;

namespace AssetConverter
{
    /// <summary>The Palette SNAM list is one flat list of sub-tracks with a null entry between two layers.</summary>
    internal static class MusicLayers
    {
        public static IReadOnlyList<(int Layer, int Ord, string SubTrack)> Split(IReadOnlyList<IFormLinkGetter<IMusicTrackGetter>>? tracks, string subject)
        {
            var rows = new List<(int, int, string)>();
            int layer = 0, ord = 0;
            foreach (var link in tracks ?? [])
            {
                if (!link.IsNull)
                {
                    rows.Add((layer, ord++, link.FormKey.ToString()));
                    continue;
                }
                if (ord == 0)
                    throw new UnsupportedEsmDataException($"{subject}: the sub-track list has an empty layer (a leading or doubled separator).");
                layer++;
                ord = 0;
            }
            if (ord == 0 && rows.Count > 0)
                throw new UnsupportedEsmDataException($"{subject}: the sub-track list ends with a separator, leaving an empty layer.");
            return rows;
        }
    }

    internal static class RegionPolygon
    {
        private const int BytesPerPoint = 2 * sizeof(float);

        /// <summary>Little-endian f32 x,y pairs in raw units.</summary>
        public static byte[] Encode(IReadOnlyList<P2Float>? points)
        {
            var bytes = new byte[(points?.Count ?? 0) * BytesPerPoint];
            for (int i = 0; i < bytes.Length / BytesPerPoint; i++)
            {
                BinaryPrimitives.WriteSingleLittleEndian(bytes.AsSpan(i * BytesPerPoint), points![i].X);
                BinaryPrimitives.WriteSingleLittleEndian(bytes.AsSpan(i * BytesPerPoint + sizeof(float)), points[i].Y);
            }
            return bytes;
        }
    }

    internal sealed class MusicTypeProcessor : BaseRecordProcessor<IMusicTypeGetter>
    {
        protected override async Task ProcessSpecificRecordAsync(IMusicTypeGetter record, string formKey, SqliteConnection conn, SqliteTransaction transaction)
        {
            if (record.Data is null)
                throw new UnsupportedEsmDataException($"MUSC {formKey} ({record.EditorID}): the record has no PNAM, so it has no priority and ducking.");
            await conn.ExecuteAsync(
                @"INSERT OR REPLACE INTO music_types (form_key, editor_id, flags, priority, ducking_centidb, fade_seconds)
                  VALUES (@FormKey, @EditorId, @Flags, @Priority, @DuckingCentidb, @FadeSeconds)",
                new
                {
                    FormKey = formKey,
                    EditorId = record.EditorID,
                    Flags = (int)record.Flags,
                    Priority = (int)record.Data.Priority,
                    DuckingCentidb = (int)MathF.Round(record.Data.DuckingDecibel * 100f),
                    FadeSeconds = record.FadeDuration,
                }, transaction);

            var tracks = (record.Tracks ?? []).Select((link, ord) => new
            {
                MusicType = formKey,
                Ord = ord,
                Track = link.IsNull
                    ? throw new UnsupportedEsmDataException($"MUSC {formKey} ({record.EditorID}): track {ord} is a null link.")
                    : link.FormKey.ToString(),
            }).ToList();
            await conn.ExecuteAsync("INSERT OR REPLACE INTO music_type_tracks (music_type, ord, track) VALUES (@MusicType, @Ord, @Track)", tracks, transaction);
        }
    }

    internal sealed class MusicTrackProcessor : BaseRecordProcessor<IMusicTrackGetter>
    {
        protected override async Task ProcessSpecificRecordAsync(IMusicTrackGetter record, string formKey, SqliteConnection conn, SqliteTransaction transaction)
        {
            string subject = $"MUST {formKey} ({record.EditorID})";
            string? anamPath = string.IsNullOrEmpty(record.TrackFilename) ? null : record.TrackFilename;
            if (record.Type == MusicTrack.TypeEnum.SingleTrack && anamPath is null)
                throw new UnsupportedEsmDataException($"{subject}: a Single track has no ANAM, so it names no file to play.");
            await conn.ExecuteAsync(
                @"INSERT OR REPLACE INTO music_tracks (form_key, editor_id, track_type, duration, fade_out, file_key, anam_path)
                  VALUES (@FormKey, @EditorId, @TrackType, @Duration, @FadeOut, @FileKey, @AnamPath)",
                new
                {
                    FormKey = formKey,
                    EditorId = record.EditorID,
                    TrackType = TrackTypeName(record.Type, subject),
                    Duration = record.Duration,
                    FadeOut = record.FadeOut,
                    FileKey = anamPath is null ? null : AudioLayout.ToAudioKey(anamPath),
                    AnamPath = anamPath,
                }, transaction);

            var layers = MusicLayers.Split(record.Tracks, subject).Select(row => new { Track = formKey, row.Layer, row.Ord, row.SubTrack });
            await conn.ExecuteAsync("INSERT OR REPLACE INTO music_track_layers (track, layer, ord, sub_track) VALUES (@Track, @Layer, @Ord, @SubTrack)", layers, transaction);

            await ConditionTable.InsertAsync(conn, transaction, "music_track_conditions", "track", formKey, record.Conditions, subject);
        }

        private static string TrackTypeName(MusicTrack.TypeEnum type, string subject) => type switch
        {
            MusicTrack.TypeEnum.SingleTrack => "Single",
            MusicTrack.TypeEnum.Palette => "Palette",
            MusicTrack.TypeEnum.SilentTrack => "Silent",
            _ => throw new UnsupportedEsmDataException($"{subject}: unknown track type {(uint)type}."),
        };
    }

    public partial class ESMLoader
    {
        private sealed record MusicTableRows(string Name, string[] Columns, IReadOnlyList<object?[]> Rows);

        // The music tables are filled by jobs that read only the links they need, so a reload of them does not run the
        // cell scan of CellsJob (placed objects, landscapes, precombined refs) or the worldspace job. They own no records rows.
        private static RecordJob MusicLinkJob(string[] tables, Func<IFallout4ModGetter, IReadOnlyList<MusicTableRows>> read) =>
            new(tables, [], (loader, context, conn, transaction) =>
            {
                foreach (var table in read(context.File))
                    BatchInsert.Execute(conn, transaction, table.Name, table.Columns, table.Rows);
                return Task.CompletedTask;
            });

        internal static RecordJob CellPlacesJob() => MusicLinkJob(["cell_places"], file =>
            [new("cell_places", ["cell", "music_type", "location"], CellPlaceRows(file))]);

        internal static RecordJob LocationMusicJob() => MusicLinkJob(["location_music"], file =>
            [new("location_music", ["location", "music_type"], LocationMusicRows(file))]);

        internal static RecordJob WorldspaceMusicJob() => MusicLinkJob(["worldspace_music"], file =>
            [new("worldspace_music", ["worldspace", "music_type"], WorldspaceMusicRows(file))]);

        internal static RecordJob RegionMusicJob() => MusicLinkJob(["region_music", "region_areas"], file =>
        [
            new("region_music", ["region", "music_type", "priority", "override", "record_order"], RegionMusicRows(file)),
            new("region_areas", ["region", "area", "edge_falloff", "points"], RegionAreaRows(file)),
        ]);

        internal static IReadOnlyList<object?[]> CellPlaceRows(IFallout4ModGetter file) => file.EnumerateMajorRecords<ICellGetter>()
            .Where(cell => !cell.Music.IsNull || !cell.Location.IsNull)
            .Select(cell => new object?[] { cell.FormKey.ToString(), LinkKey.Text(cell.Music), LinkKey.Text(cell.Location) })
            .ToList();

        internal static IReadOnlyList<object?[]> LocationMusicRows(IFallout4ModGetter file) => file.Locations
            .Where(location => !location.Music.IsNull)
            .Select(location => new object?[] { location.FormKey.ToString(), LinkKey.Text(location.Music) })
            .ToList();

        internal static IReadOnlyList<object?[]> WorldspaceMusicRows(IFallout4ModGetter file) => file.Worldspaces
            .Where(worldspace => !worldspace.Music.IsNull)
            .Select(worldspace => new object?[] { worldspace.FormKey.ToString(), LinkKey.Text(worldspace.Music) })
            .ToList();

        /// <summary><c>record_order</c> is the position among all REGN records of the file, with or without music.</summary>
        internal static IReadOnlyList<object?[]> RegionMusicRows(IFallout4ModGetter file) => file.Regions
            .Select((region, order) => (region, order))
            .Where(entry => entry.region.Sounds is { Music.IsNull: false })
            .Select(entry => new object?[]
            {
                entry.region.FormKey.ToString(), LinkKey.Text(entry.region.Sounds!.Music), (int)entry.region.Sounds.Priority,
                entry.region.Sounds.Flags.HasFlag(RegionData.RegionDataFlag.Override) ? 1 : 0, entry.order,
            })
            .ToList();

        internal static IReadOnlyList<object?[]> RegionAreaRows(IFallout4ModGetter file) => file.Regions
            .SelectMany(region => (region.RegionAreas ?? []).Select((area, index) => new object?[]
            {
                region.FormKey.ToString(), index, (int)area.EdgeFallOff, RegionPolygon.Encode(area.RegionPointListData),
            }))
            .ToList();
    }
}
