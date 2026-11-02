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
    // The loading-screen tables. A real binary plugin built from Mutagen's in-memory records is loaded through the
    // real loader; the values mirror the shapes of the base game's records.
    public sealed class EsmLoadScreenTests : IDisposable
    {
        const int DisplaysInMainMenu = 1 << 10;
        const int NoRotation = 1 << 15;
        const int AroundOrigin = 1 << 15;

        readonly TempDirectory _directory = new();
        readonly string _dbPath;

        public EsmLoadScreenTests() => _dbPath = Path.Combine(_directory.Path, "data.db");

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

        Task Load(TinyPlugin plugin) =>
            new ESMLoader(_dbPath) { Memory = MemoryBudget.Unlimited() }.LoadESMAsync(plugin.Write(_directory.Path), ct: TestContext.Current.CancellationToken);

        async Task<SqliteConnection> LoadAsync(TinyPlugin plugin)
        {
            await Load(plugin);
            return Open();
        }

        static (TinyPlugin Plugin, Static Model, Transform Camera) PluginWithModelAndCamera()
        {
            var plugin = new TinyPlugin();
            var model = plugin.AddStatic("ScreenModel", @"Meshes\LoadScreens\Model.nif");
            var camera = plugin.AddTransform("ScreenCamera", transform =>
            {
                transform.Position = new P3Float(1f, 2f, 3f);
                transform.Rotation = new P3Float(0.25f, 0.5f, 0.75f);
                transform.Scale = 1.5f;
                transform.ZoomMin = -0.5f;
                transform.ZoomMax = 1.5f;
                transform.MajorFlags = Transform.MajorFlag.AroundOrigin;
            });
            return (plugin, model, camera);
        }

        static void Describe(LoadScreen screen, Static model, Transform camera)
        {
            screen.LoadingScreenNif = new FormLink<IStaticObjectGetter>(model.FormKey);
            screen.Transform = new FormLinkNullable<ITransformGetter>(camera.FormKey);
            screen.Rotation = new() { Min = -180, Max = 180 };
        }

        static ConditionFloat Literal(Condition.Function function, CompareOperator op, float value, Condition.Flag flags = 0) =>
            new() { Data = new FunctionConditionData { Function = function }, CompareOperator = op, ComparisonValue = value, Flags = flags };

        [Fact]
        public async Task LoadScreen_StoresTheTipModelTransformRotationZoomAndFlags()
        {
            var (plugin, model, camera) = PluginWithModelAndCamera();
            plugin.AddLoadScreen("LoadTip", screen =>
            {
                Describe(screen, model, camera);
                screen.Description = "Synthetic tip text.";
                screen.Zoom = new() { Min = -0.5f, Max = 1.5f };
                screen.MajorFlags = LoadScreen.MajorFlag.NoRotation;
            });

            using var connection = await LoadAsync(plugin);
            var row = connection.QuerySingle("SELECT * FROM load_screens WHERE form_key = @key", new { key = plugin["LoadTip"] });

            Assert.Equal("Synthetic tip text.", row.tip);
            Assert.Equal((plugin["ScreenModel"], plugin["ScreenCamera"]), ((string)row.model, (string)row.transform));
            Assert.Equal((-180L, 180L), (row.rotation_min, row.rotation_max));
            Assert.Equal((-0.5, 1.5), (row.zoom_min, row.zoom_max));
            Assert.Equal((long)NoRotation, row.flags);
        }

        [Fact]
        public async Task LoadScreen_WithoutZoom_StoresNullZoomAndKeepsTheMainMenuFlag()
        {
            var (plugin, model, camera) = PluginWithModelAndCamera();
            plugin.AddLoadScreen("MenuOnly", screen =>
            {
                Describe(screen, model, camera);
                screen.MajorFlags = LoadScreen.MajorFlag.DisplaysInMainMenu;
            });

            using var connection = await LoadAsync(plugin);
            var row = connection.QuerySingle("SELECT * FROM load_screens WHERE form_key = @key", new { key = plugin["MenuOnly"] });

            Assert.Null(row.zoom_min);
            Assert.Null(row.zoom_max);
            Assert.Null(row.tip);
            Assert.Equal((long)DisplaysInMainMenu, row.flags);
        }

        [Fact]
        public async Task Transform_StoresPositionRadiansScaleFlagsAndZoom()
        {
            var (plugin, _, _) = PluginWithModelAndCamera();
            plugin.AddTransform("PlainCamera", transform => transform.Scale = 1f);

            using var connection = await LoadAsync(plugin);
            var around = connection.QuerySingle("SELECT * FROM transforms WHERE form_key = @key", new { key = plugin["ScreenCamera"] });
            var plain = connection.QuerySingle("SELECT * FROM transforms WHERE form_key = @key", new { key = plugin["PlainCamera"] });

            Assert.Equal((1.0, 2.0, 3.0), (around.pos_x, around.pos_y, around.pos_z));
            Assert.Equal((0.25, 0.5, 0.75), (around.rot_x, around.rot_y, around.rot_z));
            Assert.Equal((1.5, -0.5, 1.5, (long)AroundOrigin), (around.scale, around.zoom_min, around.zoom_max, around.flags));
            Assert.Equal(0L, plain.flags);
        }

        [Fact]
        public async Task LoadScreenConditions_KeepOrderOrFlagAndParameters()
        {
            var (plugin, model, camera) = PluginWithModelAndCamera();
            var worldspace = plugin.AddWorldspace("ConditionWorld");
            plugin.AddLoadScreen("Conditioned", screen =>
            {
                Describe(screen, model, camera);
                screen.Conditions.Add(Literal(Condition.Function.GetRandomPercent, CompareOperator.LessThanOrEqualTo, 20f, Condition.Flag.OR));
                screen.Conditions.Add(new ConditionFloat
                {
                    Data = new FunctionConditionData { Function = Condition.Function.GetInWorldspace, ParameterOneRecord = new FormLink<IFallout4MajorRecordGetter>(worldspace.FormKey) },
                    CompareOperator = CompareOperator.EqualTo,
                    ComparisonValue = 1f,
                });
            });

            using var connection = await LoadAsync(plugin);
            var rows = connection.Query("SELECT * FROM load_screen_conditions WHERE screen = @key ORDER BY ord", new { key = plugin["Conditioned"] }).ToList();

            Assert.Equal(2, rows.Count);
            Assert.Equal(((long)Condition.Function.GetRandomPercent, 5L, 1L, 20.0), (rows[0].function, rows[0].compare_op, rows[0].or_flag, rows[0].compare_value));
            Assert.Equal(((long)Condition.Function.GetInWorldspace, 0L, 0L, 1.0, plugin["ConditionWorld"]), (rows[1].function, rows[1].compare_op, rows[1].or_flag, rows[1].compare_value, rows[1].param1_record));
        }

        [Fact]
        public async Task LoadScreenCondition_WithAliasParameters_StoresTheBitInFlags()
        {
            var (plugin, model, camera) = PluginWithModelAndCamera();
            plugin.AddLoadScreen("AliasScreen", screen =>
            {
                Describe(screen, model, camera);
                screen.Conditions.Add(Literal(Condition.Function.GetRandomPercent, CompareOperator.LessThan, 50f));
                screen.Conditions.Add(Literal(Condition.Function.GetRandomPercent, CompareOperator.LessThan, 50f, Condition.Flag.ParametersUseAliases));
            });

            using var connection = await LoadAsync(plugin);
            var flags = connection.Query<long>("SELECT flags FROM load_screen_conditions WHERE screen = @key ORDER BY ord", new { key = plugin["AliasScreen"] }).ToList();

            Assert.Equal([0L, 2L], flags);
        }

        [Fact]
        public async Task LoadScreen_WithoutModel_IsRejectedNamingTheRecord()
        {
            var (plugin, _, camera) = PluginWithModelAndCamera();
            plugin.AddLoadScreen("NoModel", screen => screen.Transform = new FormLinkNullable<ITransformGetter>(camera.FormKey));

            var error = await Assert.ThrowsAsync<UnsupportedEsmDataException>(() => Load(plugin));

            Assert.Contains("NoModel", error.Message);
            Assert.Contains("NNAM", error.Message);
        }

        [Fact]
        public async Task LoadScreen_WithoutTransform_IsRejectedNamingTheRecord()
        {
            var (plugin, model, _) = PluginWithModelAndCamera();
            plugin.AddLoadScreen("NoCamera", screen => screen.LoadingScreenNif = new FormLink<IStaticObjectGetter>(model.FormKey));

            var error = await Assert.ThrowsAsync<UnsupportedEsmDataException>(() => Load(plugin));

            Assert.Contains("NoCamera", error.Message);
            Assert.Contains("TNAM", error.Message);
        }

        [Fact]
        public async Task LoadScreen_WithACameraPath_IsRejectedNamingThePath()
        {
            var (plugin, model, camera) = PluginWithModelAndCamera();
            plugin.AddLoadScreen("PathScreen", screen =>
            {
                Describe(screen, model, camera);
                screen.CameraPath = @"Meshes\Cameras\Path.nif";
            });

            var error = await Assert.ThrowsAsync<UnsupportedEsmDataException>(() => Load(plugin));

            Assert.Contains("PathScreen", error.Message);
            Assert.Contains("Path.nif", error.Message);
        }
    }
}
