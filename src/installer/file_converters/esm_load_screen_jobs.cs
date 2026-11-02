using Dapper;
using Microsoft.Data.Sqlite;
using Mutagen.Bethesda.Fallout4;

namespace AssetConverter
{
    internal sealed class LoadScreenProcessor : BaseRecordProcessor<ILoadScreenGetter>
    {
        protected override async Task ProcessSpecificRecordAsync(ILoadScreenGetter record, string formKey, SqliteConnection conn, SqliteTransaction transaction)
        {
            string subject = $"LSCR {formKey} ({record.EditorID})";
            if (!string.IsNullOrEmpty(record.CameraPath))
                throw new UnsupportedEsmDataException($"{subject}: the camera path '{record.CameraPath}' (MOD2) is not supported.");
            string model = LinkKey.Text(record.LoadingScreenNif)
                ?? throw new UnsupportedEsmDataException($"{subject}: the record has no NNAM, so it names no model to show.");
            string transform = LinkKey.Text(record.Transform)
                ?? throw new UnsupportedEsmDataException($"{subject}: the record has no TNAM, so it names no camera transform.");

            await conn.ExecuteAsync(
                @"INSERT OR REPLACE INTO load_screens (form_key, editor_id, tip, model, flags, transform, rotation_min, rotation_max, zoom_min, zoom_max)
                  VALUES (@FormKey, @EditorId, @Tip, @Model, @Flags, @Transform, @RotationMin, @RotationMax, @ZoomMin, @ZoomMax)",
                new
                {
                    FormKey = formKey,
                    EditorId = record.EditorID,
                    Tip = record.Description?.String,
                    Model = model,
                    Flags = (int)record.MajorFlags,
                    Transform = transform,
                    RotationMin = record.Rotation?.Min,
                    RotationMax = record.Rotation?.Max,
                    ZoomMin = record.Zoom?.Min,
                    ZoomMax = record.Zoom?.Max,
                }, transaction);

            await ConditionTable.InsertAsync(conn, transaction, "load_screen_conditions", "screen", formKey, record.Conditions, subject);
        }
    }

    internal sealed class TransformProcessor : BaseRecordProcessor<ITransformGetter>
    {
        protected override async Task ProcessSpecificRecordAsync(ITransformGetter record, string formKey, SqliteConnection conn, SqliteTransaction transaction)
        {
            await conn.ExecuteAsync(
                @"INSERT OR REPLACE INTO transforms (form_key, editor_id, pos_x, pos_y, pos_z, rot_x, rot_y, rot_z, scale, flags, zoom_min, zoom_max)
                  VALUES (@FormKey, @EditorId, @PosX, @PosY, @PosZ, @RotX, @RotY, @RotZ, @Scale, @Flags, @ZoomMin, @ZoomMax)",
                new
                {
                    FormKey = formKey,
                    EditorId = record.EditorID,
                    PosX = record.Position.X,
                    PosY = record.Position.Y,
                    PosZ = record.Position.Z,
                    RotX = record.Rotation.X,
                    RotY = record.Rotation.Y,
                    RotZ = record.Rotation.Z,
                    Scale = record.Scale,
                    Flags = (int)record.MajorFlags,
                    ZoomMin = record.ZoomMin,
                    ZoomMax = record.ZoomMax,
                }, transaction);
        }
    }
}
