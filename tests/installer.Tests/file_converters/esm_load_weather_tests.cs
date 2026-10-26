using System.Text.Json;
using AssetConverter;
using AssetConverter.Pipeline;
using Dapper;
using InstallerTests.Support;
using Microsoft.Data.Sqlite;
using Mutagen.Bethesda.Fallout4;
using Mutagen.Bethesda.Plugins;
using Xunit;

namespace InstallerTests
{
    // Weather, god rays, climate and shader particle geometry columns the engine reads (docs/installer/database.md).
    [Collection(FixtureEsm.Name)]
    public class EsmLoadWeatherTests(LoadedFixtureDatabase db)
    {
        JsonElement Json(string table, string column, string editorId) =>
            JsonDocument.Parse(db.QuerySingle<string>($"SELECT {column} FROM {table} WHERE form_key = @key", new { key = db.KeyOf(editorId) })).RootElement;

        JsonElement Clouds() => Json("weather", "clouds", "FixtureWeather");

        JsonElement WeatherData(string editorId = "FixtureWeather") => Json("weather", "data", editorId);

        [Fact]
        public void Weather_Clouds_HaveThirtyTwoLayersAndTwentyNineTexturesAndNothingElse()
        {
            var clouds = Clouds();

            Assert.Equal(32, clouds.GetProperty("layers").GetArrayLength());
            Assert.Equal(29, clouds.GetProperty("textures").GetArrayLength());
            Assert.Equal(["layers", "textures"], clouds.EnumerateObject().Select(property => property.Name).Order());
        }

        [Fact]
        public void Weather_CloudTextures_AreIndexedBySubrecordSlot()
        {
            var textures = Clouds().GetProperty("textures");

            Assert.Equal(WeatherFixture.CloudTexture, textures[WeatherFixture.TexturedCloudSlot].GetString());
            Assert.Equal(1, textures.EnumerateArray().Count(texture => texture.ValueKind == JsonValueKind.String));
        }

        [Fact]
        public void Weather_CloudLayer_StoresEnabledSpeedsColorsAndAlphasPerTimeOfDay()
        {
            var layer = Clouds().GetProperty("layers")[WeatherFixture.ConfiguredCloudLayer];
            var colors = layer.GetProperty("colors");
            var alphas = layer.GetProperty("alphas");

            Assert.True(layer.GetProperty("enabled").GetBoolean());
            Assert.Equal(WeatherFixture.CloudXSpeed, layer.GetProperty("x_speed").GetDouble(), 2);
            Assert.Equal(WeatherFixture.CloudYSpeed, layer.GetProperty("y_speed").GetDouble(), 2);
            Assert.Equal([WeatherFixture.CloudDayRed, 20, 30, 255], colors.GetProperty("Day").EnumerateArray().Select(c => c.GetInt32()));
            Assert.Equal([4, 5, 6, 255], colors.GetProperty("LateSunset").EnumerateArray().Select(c => c.GetInt32()));
            Assert.Equal(WeatherFixture.CloudDayAlpha, alphas.GetProperty("Day").GetSingle());
            Assert.Equal(WeatherFixture.CloudEarlySunriseAlpha, alphas.GetProperty("EarlySunrise").GetSingle());
        }

        [Fact]
        public void Weather_CloudLayer_UsesTheTimeOfDayNamesOfTheImageSpacesColumn()
        {
            var imageSpaces = Json("weather", "image_spaces", "FixtureWeather");
            string[] slots = [.. imageSpaces.EnumerateObject().Select(slot => slot.Name).Order()];
            var layer = Clouds().GetProperty("layers")[0];

            Assert.Equal(8, slots.Length);
            Assert.Equal(slots, layer.GetProperty("colors").EnumerateObject().Select(slot => slot.Name).Order());
            Assert.Equal(slots, layer.GetProperty("alphas").EnumerateObject().Select(slot => slot.Name).Order());
        }

        [Fact]
        public void Weather_Data_StoresPercentsAsFractions()
        {
            var data = WeatherData();

            Assert.Equal(WeatherFixture.WindSpeed, data.GetProperty("wind_speed").GetDouble(), 6);
            Assert.Equal(WeatherFixture.WindTurbulence, data.GetProperty("wind_turbulence").GetDouble(), 6);
            Assert.Equal(WeatherFixture.SunGlare, data.GetProperty("sun_glare").GetDouble(), 6);
            Assert.Equal(WeatherFixture.ThunderFrequency, data.GetProperty("thunder_frequency").GetDouble(), 6);
        }

        [Fact]
        public void Weather_Data_StoresAuthoredFlagsLightningColorAndMultipliers()
        {
            var data = WeatherData();

            Assert.Equal(2 | 64, data.GetProperty("flags").GetInt32());
            Assert.Equal([WeatherFixture.LightningRed, 200, 100], data.GetProperty("lightning_color").EnumerateArray().Select(c => c.GetInt32()));
            Assert.Equal(WeatherFixture.VolatilityMult, data.GetProperty("volatility_mult").GetSingle());
            Assert.Equal(JsonValueKind.Null, data.GetProperty("visibility_mult").ValueKind);
        }

        [Fact]
        public void Weather_Data_RawFieldsAreTheAuthoredBytes()
        {
            var data = WeatherData();

            Assert.Equal(WeatherFixture.WindDirectionRaw, data.GetProperty("wind_direction_raw").GetInt32());
            Assert.Equal(WeatherFixture.WindDirectionRangeRaw, data.GetProperty("wind_direction_range_raw").GetInt32());
            Assert.Equal(WeatherFixture.TransDeltaRaw, data.GetProperty("trans_delta_raw").GetInt32());
        }

        [Fact]
        public void Weather_Data_RawFieldsInvertMutagenScalingOfBytesWrittenByTheGame()
        {
            var data = WeatherData("RawDataWeather");

            Assert.Equal(25, data.GetProperty("trans_delta_raw").GetInt32());
            Assert.Equal(180, data.GetProperty("wind_direction_raw").GetInt32());
            Assert.Equal(90, data.GetProperty("wind_direction_range_raw").GetInt32());
            Assert.Equal(1.0, data.GetProperty("wind_speed").GetDouble());
            Assert.Equal(1.0, data.GetProperty("thunder_frequency").GetDouble());
            Assert.Equal(127 / 255.0, data.GetProperty("wind_turbulence").GetDouble(), 6);
        }

        [Fact]
        public void Weather_Data_WithoutTurbulenceByte_HasNullTurbulenceAndKeepsTheOtherFields()
        {
            var data = WeatherData("ShortDataWeather");

            Assert.Equal(JsonValueKind.Null, data.GetProperty("wind_turbulence").ValueKind);
            Assert.Equal(180, data.GetProperty("wind_direction_raw").GetInt32());
        }

        [Fact]
        public void Weather_Sounds_NameTheRawKindAndKeepTheSoundDescriptorFormKey()
        {
            var sounds = Json("weather", "sounds", "FixtureWeather").EnumerateArray().ToList();

            Assert.Equal(WeatherFixture.SoundKinds.Select(sound => sound.Kind), sounds.Select(sound => sound.GetProperty("kind").GetString()));
            Assert.All(sounds, sound => Assert.Equal(db.KeyOf("ThunderSound"), sound.GetProperty("sound").GetString()));
            Assert.All(sounds, sound => Assert.False(sound.TryGetProperty("type", out _)));
        }

        [Fact]
        public void WeatherSoundKind_WithUnknownRawValue_FailsNamingTheSubjectAndTheValue()
        {
            var error = Assert.Throws<UnsupportedEsmDataException>(() => WeatherJson.SoundKind(4, "WTHR subject"));

            Assert.Contains("WTHR subject", error.Message);
            Assert.Contains("4", error.Message);
        }

        [Fact]
        public void Weather_WithoutSounds_HasAnEmptySoundList()
        {
            Assert.Equal(0, Json("weather", "sounds", "ShortDataWeather").GetArrayLength());
        }

        [Fact]
        public void Weather_GodRays_NameTheGodRaysFormKeyPerTimeOfDayAndNullForEmptySlots()
        {
            var godRays = Json("weather", "god_rays", "FixtureWeather");
            var imageSpaceSlots = Json("weather", "image_spaces", "FixtureWeather").EnumerateObject().Select(slot => slot.Name).Order();

            Assert.Equal(imageSpaceSlots, godRays.EnumerateObject().Select(slot => slot.Name).Order());
            Assert.Equal(db.KeyOf("FixtureGodRays"), godRays.GetProperty("Day").GetString());
            Assert.Equal(db.KeyOf("FixtureGodRays"), godRays.GetProperty("LateSunset").GetString());
            Assert.Equal(6, godRays.EnumerateObject().Count(slot => slot.Value.ValueKind == JsonValueKind.Null));
        }

        [Fact]
        public void Weather_WithoutGodRaysSubrecord_HasNullGodRays()
        {
            Assert.Null(db.QuerySingle<string?>("SELECT god_rays FROM weather WHERE form_key = @key", new { key = db.KeyOf("RawDataWeather") }));
        }

        [Fact]
        public void GodRays_Data_StoresColorsAsBytesAndTheOtherFieldsAsFloats()
        {
            var data = Json("god_rayses", "data", "FixtureGodRays");

            Assert.Equal(
                ["air_color", "air_color_scale", "back_color", "back_color_scale", "back_phase", "forward_color", "forward_color_scale", "forward_phase", "intensity"],
                data.EnumerateObject().Select(property => property.Name).Order());
            Assert.Equal([WeatherFixture.GodRaysBackRed, 128, 0], data.GetProperty("back_color").EnumerateArray().Select(c => c.GetInt32()));
            Assert.Equal([1, 2, 3], data.GetProperty("forward_color").EnumerateArray().Select(c => c.GetInt32()));
            Assert.Equal([10, 20, 30], data.GetProperty("air_color").EnumerateArray().Select(c => c.GetInt32()));
            Assert.Equal(WeatherFixture.GodRaysBackColorScale, data.GetProperty("back_color_scale").GetSingle());
            Assert.Equal(WeatherFixture.GodRaysForwardColorScale, data.GetProperty("forward_color_scale").GetSingle());
            Assert.Equal(WeatherFixture.GodRaysAirColorScale, data.GetProperty("air_color_scale").GetSingle());
            Assert.Equal(WeatherFixture.GodRaysIntensity, data.GetProperty("intensity").GetSingle());
            Assert.Equal(WeatherFixture.GodRaysBackPhase, data.GetProperty("back_phase").GetSingle());
            Assert.Equal(WeatherFixture.GodRaysForwardPhase, data.GetProperty("forward_phase").GetSingle());
        }

        [Fact]
        public void Climate_Volatility_IsTheTnamByte()
        {
            Assert.Equal(60L, db.QuerySingle<long>("SELECT volatility FROM climates WHERE form_key = @key", new { key = db.KeyOf("VolatileClimate") }));
            Assert.Equal(0L, db.QuerySingle<long>("SELECT volatility FROM climates WHERE form_key = @key", new { key = db.KeyOf("FixtureClimate") }));
        }

        [Fact]
        public void ShaderParticleGeometry_StoresDataJsonAndParticleTexture()
        {
            var data = Json("shader_particle_geometries", "data", "SnowGeometry");

            Assert.Equal(1, data.GetProperty("type").GetInt32());
            Assert.Equal(1.5, data.GetProperty("gravity_velocity").GetDouble());
            Assert.Equal(0.25, data.GetProperty("particle_size_x").GetDouble());
            Assert.Equal(4, data.GetProperty("subtextures_x").GetInt32());
            Assert.Equal(2, data.GetProperty("subtextures_y").GetInt32());
            Assert.Equal(4096, data.GetProperty("box_size").GetInt64());
            Assert.Equal(0.5, data.GetProperty("particle_density").GetDouble());
        }

        [Fact]
        public void ShaderParticleGeometry_WithoutMaterialsRoot_KeepsTheAuthoredMaterialAndNoTexture()
        {
            var row = db.QuerySingle<(string? Material, string? Texture)>(
                "SELECT particle_material AS Material, particle_texture AS Texture FROM shader_particle_geometries WHERE form_key = @key",
                new { key = db.KeyOf("SnowGeometry") });

            Assert.Equal(WeatherFixture.ParticleMaterial, row.Material);
            Assert.Null(row.Texture);
        }

        [Theory]
        [InlineData(BaseMaterialFile.AlphaBlendModeType.None, false, "one", "one")]
        [InlineData(BaseMaterialFile.AlphaBlendModeType.Unknown, false, "src_alpha", "inv_src_alpha")]
        [InlineData(BaseMaterialFile.AlphaBlendModeType.Standard, true, "src_alpha", "inv_src_alpha")]
        [InlineData(BaseMaterialFile.AlphaBlendModeType.Additive, true, "src_alpha", "one")]
        [InlineData(BaseMaterialFile.AlphaBlendModeType.Multiplicative, true, "dst_color", "zero")]
        public void ParticleMaterialData_BlendFactors_AreTheStoredFactorsOfTheMaterialBlendMode(
            BaseMaterialFile.AlphaBlendModeType mode, bool enabled, string source, string destination)
        {
            var material = EffectMaterialParams.FromBgem(new BGEM { AlphaBlendMode = mode });

            var data = ShaderParticleGeometryJson.MaterialData(material);

            Assert.Equal(enabled, (bool)data["blend_enabled"]!);
            Assert.Equal(source, (string)data["blend_src"]!);
            Assert.Equal(destination, (string)data["blend_dst"]!);
        }

        [Theory]
        [InlineData(0)]
        [InlineData(1)]
        [InlineData(127)]
        [InlineData(254)]
        [InlineData(255)]
        public void RawByteConversions_InvertMutagenScalingForEveryByte(int raw)
        {
            Assert.Equal(raw, WeatherJson.RawWindDirection(raw / 360f));
            Assert.Equal(raw, WeatherJson.RawWindDirectionRange(raw / 180f));
            Assert.Equal(raw, WeatherJson.RawTransDelta(raw * 4f));
        }
    }

    [Collection("Console redirection")]
    public sealed class WeatherSoundFailureTests : IDisposable
    {
        readonly TempDirectory _directory = new();
        readonly TextWriter _originalOut = Console.Out;
        readonly StringWriter _output = new();

        public WeatherSoundFailureTests() => Console.SetOut(_output);

        public void Dispose()
        {
            Console.SetOut(_originalOut);
            SqliteConnection.ClearAllPools();
            _directory.Dispose();
        }

        [Fact]
        public async Task Load_WeatherWithAnUnknownSoundType_FailsNamingTheRecordAndTheValue()
        {
            const uint unknownType = 4;
            var plugin = new TinyPlugin();
            plugin.AddSoundDescriptor("Rumble", descriptor => descriptor.Data = new SoundDescriptorStandardData());
            plugin.AddWeather("BrokenWeather", weather => weather.Sounds.Add(new WeatherSound
            {
                Sound = new FormLink<ISoundDescriptorGetter>(plugin.FormKeyOf("Rumble")),
                Type = (WeatherSound.TypeEnum)unknownType
            }));
            var loader = new ESMLoader(Path.Combine(_directory.Path, "data.db")) { Memory = MemoryBudget.Unlimited() };

            var error = await Assert.ThrowsAsync<UnsupportedEsmDataException>(() =>
                loader.LoadESMAsync(plugin.Write(_directory.Path), ct: TestContext.Current.CancellationToken));

            Assert.Contains($"WTHR {plugin["BrokenWeather"]} (BrokenWeather)", error.Message);
            Assert.Contains($"unknown weather sound type {unknownType}", error.Message);
        }
    }

    [Collection("Console redirection")]
    public sealed class GodRaysShortDataTests : IDisposable
    {
        readonly TempDirectory _directory = new();
        readonly TextWriter _originalOut = Console.Out;
        readonly StringWriter _output = new();

        public GodRaysShortDataTests() => Console.SetOut(_output);

        public void Dispose()
        {
            Console.SetOut(_originalOut);
            SqliteConnection.ClearAllPools();
            _directory.Dispose();
        }

        [Theory]
        [InlineData(0)]
        [InlineData(20)]
        [InlineData(59)]
        public async Task Load_GodRaysWithShortData_StoresNullDataAndWarnsNamingTheRecord(int length)
        {
            var plugin = new TinyPlugin();
            plugin.AddGodRays("ShortGodRays", _ => { });
            string esmPath = plugin.Write(_directory.Path);
            RawEsmBuilder.ReplaceSubrecord(esmPath, "GDRY", plugin.FormKeyOf("ShortGodRays").ID, "DATA", new byte[length]);
            string dbPath = Path.Combine(_directory.Path, "data.db");

            await new ESMLoader(dbPath) { Memory = MemoryBudget.Unlimited() }.LoadESMAsync(esmPath, ct: TestContext.Current.CancellationToken);

            using var connection = new SqliteConnection(new SqliteConnectionStringBuilder { DataSource = dbPath, Pooling = false }.ToString());
            connection.Open();
            var row = connection.QuerySingle<(string? EditorId, string? Data)>(
                "SELECT editor_id AS EditorId, data AS Data FROM god_rayses WHERE form_key = @key", new { key = plugin["ShortGodRays"] });
            Assert.Equal("ShortGodRays", row.EditorId);
            Assert.Null(row.Data);
            Assert.Contains("WARNING: 1 unreadable reads at GDRY DATA (first: ShortGodRays", _output.ToString());
        }
    }

    [Collection("Console redirection")]
    public sealed class ShaderParticleMaterialTests : IDisposable
    {
        readonly TempDirectory _directory = new();
        readonly TextWriter _originalOut = Console.Out;
        readonly TextWriter _originalError = Console.Error;
        readonly StringWriter _output = new();
        readonly StringWriter _errors = new();

        public ShaderParticleMaterialTests()
        {
            Console.SetOut(_output);
            Console.SetError(_errors);
        }

        public void Dispose()
        {
            Console.SetOut(_originalOut);
            Console.SetError(_originalError);
            SqliteConnection.ClearAllPools();
            _directory.Dispose();
        }

        // The draw state of the vanilla rain material: additive SrcAlpha/One, no depth write, effect lighting at full influence.
        static BgemFields RainMaterial() => new()
        {
            Base = new MaterialBaseFields { BlendA = 1, BlendB = 6, BlendC = 0, ZBufferWrite = false },
            BaseTexture = @"textures\" + WeatherFixture.ParticleTexture,
            BaseColor = (205 / 255f, 213 / 255f, 220 / 255f),
            EffectLighting = true,
            LightingInfluence = 1f,
        };

        async Task<(string? Material, string? Texture, string? MaterialData)> LoadAsync(string? materialFile, bool withMaterialsRoot = true, byte[]? materialBytes = null)
        {
            string materialsRoot = Path.Combine(_directory.Path, "staging");
            if (materialFile != null)
            {
                string path = Path.Combine(materialsRoot, "Materials", materialFile);
                Directory.CreateDirectory(Path.GetDirectoryName(path)!);
                File.WriteAllBytes(path, materialBytes ?? RainMaterial().Build(21));
            }
            var plugin = new TinyPlugin();
            plugin.AddShaderParticleGeometry("RainGeometry", geometry => geometry.ParticleTexture = WeatherFixture.ParticleMaterial);
            string dbPath = Path.Combine(_directory.Path, "data.db");
            await new ESMLoader(dbPath, withMaterialsRoot ? materialsRoot : null) { Memory = MemoryBudget.Unlimited() }.LoadESMAsync(plugin.Write(_directory.Path), ct: TestContext.Current.CancellationToken);

            using var connection = new SqliteConnection(new SqliteConnectionStringBuilder { DataSource = dbPath, Pooling = false }.ToString());
            connection.Open();
            return connection.QuerySingle<(string? Material, string? Texture, string? MaterialData)>(
                "SELECT particle_material AS Material, particle_texture AS Texture, particle_material_data AS MaterialData FROM shader_particle_geometries WHERE form_key = @key",
                new { key = plugin["RainGeometry"] });
        }

        [Fact]
        public async Task ParticleMaterial_PresentInMaterialsRoot_ResolvesToItsBaseTextureRelativeToTheTexturesFolder()
        {
            var row = await LoadAsync(@"effects\FXWeatherRaindrops.BGEM");

            Assert.Equal(WeatherFixture.ParticleMaterial, row.Material);
            Assert.Equal(WeatherFixture.ParticleTexture, row.Texture);
            Assert.DoesNotContain("(RainGeometry): could not resolve", _errors.ToString());
            Assert.Contains("Particle geometries resolved via BGEM material: 1", _output.ToString());
            Assert.DoesNotContain("Particle geometries with unresolvable material", _output.ToString());
        }

        [Fact]
        public async Task ParticleMaterial_PresentInMaterialsRoot_StoresItsBlendBaseColorDepthWriteAndEffectLighting()
        {
            var row = await LoadAsync(@"effects\FXWeatherRaindrops.BGEM");

            var data = JsonDocument.Parse(row.MaterialData!).RootElement;
            Assert.Equal(
                ["base_color", "blend_dst", "blend_enabled", "blend_src", "depth_write", "effect_lighting", "lighting_influence"],
                data.EnumerateObject().Select(property => property.Name).Order());
            Assert.True(data.GetProperty("blend_enabled").GetBoolean());
            Assert.Equal("src_alpha", data.GetProperty("blend_src").GetString());
            Assert.Equal("one", data.GetProperty("blend_dst").GetString());
            Assert.Equal([205, 213, 220], data.GetProperty("base_color").EnumerateArray().Select(c => c.GetInt32()));
            Assert.False(data.GetProperty("depth_write").GetBoolean());
            Assert.True(data.GetProperty("effect_lighting").GetBoolean());
            Assert.Equal(1.0, data.GetProperty("lighting_influence").GetDouble());
            Assert.DoesNotContain("as an effect material", _errors.ToString());
            Assert.DoesNotContain("without readable effect material data", _output.ToString());
        }

        [Fact]
        public async Task ParticleMaterial_ThatIsALightingMaterial_KeepsItsTextureAndWarnsThatItHasNoEffectData()
        {
            byte[] bgsm = new BgsmFields { Diffuse = WeatherFixture.ParticleTexture }.Build(2);

            var row = await LoadAsync(@"effects\FXWeatherRaindrops.BGEM", materialBytes: bgsm);

            Assert.Equal(WeatherFixture.ParticleTexture, row.Texture);
            Assert.Null(row.MaterialData);
            Assert.Contains("(RainGeometry): could not read particle material `" + WeatherFixture.ParticleMaterial + "` as an effect material", _errors.ToString());
            Assert.Contains("Particle geometries without readable effect material data: 1", _output.ToString());
        }

        [Fact]
        public async Task ParticleMaterial_MissingFromMaterialsRoot_KeepsTheMaterialAndLeavesTheTextureNull()
        {
            var row = await LoadAsync(null);

            Assert.Equal(WeatherFixture.ParticleMaterial, row.Material);
            Assert.Null(row.Texture);
            Assert.Contains("CONVERSION ERROR SPGD", _errors.ToString());
            Assert.Contains("(RainGeometry): could not resolve particle material `" + WeatherFixture.ParticleMaterial + "`", _errors.ToString());
            Assert.Contains("Particle geometries with unresolvable material: 1", _output.ToString());
            Assert.Null(row.MaterialData);
            Assert.Contains("(RainGeometry): could not read particle material `" + WeatherFixture.ParticleMaterial + "` as an effect material", _errors.ToString());
            Assert.Contains("Particle geometries without readable effect material data: 1", _output.ToString());
        }

        [Fact]
        public async Task ParticleMaterial_WithoutMaterialsRoot_KeepsTheMaterialAndReportsNothing()
        {
            var row = await LoadAsync(@"effects\FXWeatherRaindrops.BGEM", withMaterialsRoot: false);

            Assert.Equal(WeatherFixture.ParticleMaterial, row.Material);
            Assert.Null(row.Texture);
            Assert.DoesNotContain("(RainGeometry): could not resolve", _errors.ToString());
            Assert.DoesNotContain("Particle geometries with unresolvable material", _output.ToString());
            Assert.Null(row.MaterialData);
            Assert.DoesNotContain("as an effect material", _errors.ToString());
        }
    }

    public sealed class WeatherColumnMigrationTests : IDisposable
    {
        readonly TempDirectory _directory = new();

        public void Dispose()
        {
            SqliteConnection.ClearAllPools();
            _directory.Dispose();
        }

        string DatabasePath => Path.Combine(_directory.Path, "data.db");

        [Fact]
        public async Task Initialize_DatabaseWithOldWeatherClimateParticleAndGodRaysTables_AddsTheNewColumnsAndKeepsRows()
        {
            using (var old = new SqliteConnection($"Data Source={DatabasePath}"))
            {
                old.Open();
                old.Execute("CREATE TABLE weather (form_key TEXT PRIMARY KEY, editor_id TEXT, precipitation TEXT, fog TEXT, colors TEXT, image_spaces TEXT, sounds TEXT, god_rays TEXT, directional_ambient TEXT)");
                old.Execute("CREATE TABLE climates (form_key TEXT PRIMARY KEY, editor_id TEXT, sun_texture TEXT, sun_glare_texture TEXT, data TEXT)");
                old.Execute("CREATE TABLE shader_particle_geometries (form_key TEXT PRIMARY KEY, editor_id TEXT)");
                old.Execute("CREATE TABLE god_rayses (form_key TEXT PRIMARY KEY, editor_id TEXT)");
                old.Execute("INSERT INTO weather (form_key, editor_id) VALUES ('000001:A.esm', 'OldWeather')");
                old.Execute("INSERT INTO climates (form_key, editor_id) VALUES ('000002:A.esm', 'OldClimate')");
            }
            SqliteConnection.ClearAllPools();
            using var manager = new DatabaseManager(DatabasePath);

            await manager.InitializeAsync(bulkLoad: false);

            Assert.Superset(new HashSet<string> { "clouds", "data" }, Columns(manager, "weather"));
            Assert.Contains("volatility", Columns(manager, "climates"));
            Assert.Superset(new HashSet<string> { "data", "particle_material", "particle_texture", "particle_material_data" }, Columns(manager, "shader_particle_geometries"));
            Assert.Contains("data", Columns(manager, "god_rayses"));
            Assert.Equal("OldWeather", manager.Connection.ExecuteScalar<string>("SELECT editor_id FROM weather"));
            Assert.Null(manager.Connection.ExecuteScalar<long?>("SELECT volatility FROM climates"));
        }

        static HashSet<string> Columns(DatabaseManager manager, string table) =>
            manager.Connection.Query<string>("SELECT name FROM pragma_table_info(@table)", new { table }).ToHashSet();
    }
}
