using AssetConverter;
using Dapper;
using Microsoft.Data.Sqlite;
using Mutagen.Bethesda.Fallout4;
using Mutagen.Bethesda.Plugins;
using Mutagen.Bethesda.Plugins.Records;
using Mutagen.Bethesda.Strings;
using Noggog;
using Xunit;

namespace InstallerTests
{
    /// <summary>
    /// Builds the tiny plugin once, loads it with the real <see cref="ESMLoader"/> into a temp database
    /// created from the real esm_schema.sql, and lets tests query the rows the engine would read.
    /// </summary>
    public sealed class LoadedFixtureDatabase : IAsyncLifetime
    {
        const int DeletedFlag = 0x20;
        public const string ModelOfWall = @"Architecture\Wall01.nif";

        static LoadedFixtureDatabase() => DefaultTypeMap.MatchNamesWithUnderscores = true;

        readonly TempDirectory _directory = new();
        internal TinyPlugin Plugin { get; } = new();

        public string DatabasePath => Path.Combine(_directory.Path, "data.db");

        public string EsmPath { get; private set; } = "";

        public async ValueTask InitializeAsync()
        {
            DescribeFixtureRecords();
            EsmPath = Plugin.Write(_directory.Path);
            RawEsmBuilder.AddRecordFlags(EsmPath, "REFR", Plugin.FormKeyOf("DeletedRef").ID, DeletedFlag);
            ReplaceImageSpaceDof("FixtureImageSpace", [1f, -2500f, 3500f, FixtureImageSpaceMode, FixtureVignetteRadius, FixtureVignetteStrength]);
            ReplaceImageSpaceDof("FixtureImageSpaceWithoutVignette", [0.5f, 100f, 200f, FixtureShortImageSpaceMode]);
            AppendWaterCurrents("CELL", "FixtureExterior", CellCurrents);
            AppendWaterCurrents("REFR", "WaterRef", PlacedCurrents);
            await NavMeshFixture.Loader(DatabasePath).LoadESMAsync(EsmPath, ct: TestContext.Current.CancellationToken);
        }

        /// <summary>Mutagen rewrites the IMGS DNAM from its own model; the loader must read the bytes as the game does.</summary>
        void ReplaceImageSpaceDof(string editorId, float[] dnamFloats) =>
            RawEsmBuilder.ReplaceSubrecord(EsmPath, "IMGS", Plugin.FormKeyOf(editorId).ID, "DNAM",
                dnamFloats.SelectMany(BitConverter.GetBytes).ToArray());

        void AppendWaterCurrents(string signature, string editorId, float[][] currents) =>
            RawEsmBuilder.AppendSubrecords(EsmPath, signature, Plugin.FormKeyOf(editorId).ID,
                RawEsmBuilder.Subrecord("XWCN", BitConverter.GetBytes((uint)currents.Length)),
                RawEsmBuilder.Subrecord("XWCU", currents.SelectMany(entry => entry.SelectMany(BitConverter.GetBytes)).ToArray()));

        public ValueTask DisposeAsync()
        {
            SqliteConnection.ClearAllPools();
            _directory.Dispose();
            return ValueTask.CompletedTask;
        }

        public string KeyOf(string editorId) => Plugin[editorId];

        public IReadOnlyList<T> Query<T>(string sql, object? parameters = null)
        {
            using var connection = Open();
            return connection.Query<T>(sql, parameters).ToList();
        }

        public T QuerySingle<T>(string sql, object? parameters = null)
        {
            using var connection = Open();
            return connection.QuerySingle<T>(sql, parameters);
        }

        SqliteConnection Open()
        {
            var connection = new SqliteConnection(new SqliteConnectionStringBuilder
            {
                DataSource = DatabasePath,
                Mode = SqliteOpenMode.ReadOnly,
                Pooling = false,
            }.ToString());
            connection.Open();
            return connection;
        }

        void DescribeFixtureRecords()
        {
            var plugin = Plugin;
            plugin.AddClimate("FixtureClimate");
            plugin.AddWorldspace("FixtureWorld").Climate.FormKey = plugin.FormKeyOf("FixtureClimate");
            DescribeWater(plugin);
            plugin.AddImageSpace("FixtureImageSpace", imageSpace =>
            {
                imageSpace.DepthOfFieldStrength = 1f;
                imageSpace.DepthOfFieldDistance = -2500f;
                imageSpace.DepthOfFieldRange = 3500f;
            });
            plugin.AddImageSpace("FixtureImageSpaceWithoutVignette", imageSpace =>
            {
                imageSpace.DepthOfFieldStrength = 0.5f;
                imageSpace.DepthOfFieldDistance = 100f;
                imageSpace.DepthOfFieldRange = 200f;
            });
            plugin.AddStatic("WallStatic", ModelOfWall);
            plugin.AddStatic("MarkerStatic", @"Markers\Marker.nif", TinyPlugin.MarkerFlag);
            plugin.AddDoor("DoorBase", @"Doors\Door01.nif");
            plugin.AddLight("LampLight", light =>
            {
                light.Radius = 512;
                light.Color = TinyPlugin.Rgb(255, 128, 64);
                light.FadeValue = 1.5f;
                light.FalloffExponent = 2f;
                light.FOV = 90f;
            });
            plugin.AddMaterialSwap("RedSwap");
            DescribeWeapons(plugin);
            plugin.AddMaterialType("StoneMaterial", material =>
            {
                material.Name = "Stone";
                material.Buoyancy = -1f;
                material.Flags = MaterialType.Flag.ArrowsStick;
            });
            plugin.AddMaterialType("ClothMaterial", material => material.Name = "Cloth");
            plugin.AddMaterialType("UnnamedMaterial");
            plugin.AddLandscapeTexture("GrassTexture");
            DescribeGrass(plugin);
            DescribeWeather(plugin);
            DescribeExteriorRefs(plugin);
            DescribeLandscape(plugin);
            DescribeInterior(plugin);
            NavMeshFixture.Describe(plugin);
            DescribePipboyRecords(plugin);
        }

        public const string FixtureMapImage = @"Interface\Pip-Boy\FixtureMap_d.dds";
        public const string MarkerName = "Fixture Vault";
        public const int MarkerType = 15;
        public const int MarkerFlags = 0x03;
        public static readonly P3Float MarkerPosition = new(-40789.5f, -20621.25f, 2789.75f);
        public const string StrengthName = "Strength";
        public const string StrengthDescription = "Strength is a measure of your raw physical power.";
        public const string PerkMovie = @"Components\VaultBoys\Perks\Fixture.swf";

        static void DescribePipboyRecords(TinyPlugin plugin)
        {
            plugin.Worldspace.MapImage = FixtureMapImage;
            plugin.PlaceInPersistentCell("FixtureMarkerRef", "MarkerStatic", r =>
            {
                r.Position = MarkerPosition;
                r.MapMarker = new PlacedObjectMapMarker
                {
                    Name = new TranslatedString(Language.English, MarkerName),
                    Type = (PlacedObjectMapMarker.Types)MarkerType,
                    Flags = (PlacedObjectMapMarker.Flag)MarkerFlags,
                };
            });

            plugin.AddActorValue("Strength", StrengthName, StrengthDescription);
            foreach (var editorId in new[] { "Perception", "Endurance", "Charisma", "Intelligence", "Agility", "Luck" })
                plugin.AddActorValue(editorId);
            var strengthAtLeast6 = () => plugin.ActorValueCondition("Strength", CompareOperator.GreaterThanOrEqualTo, 6f);

            var cyclicFirst = plugin.AddPerk("FixtureCyclicPerk1", perk =>
            {
                perk.Level = 0;
                perk.NumRanks = 2;
                perk.Playable = true;
                perk.Swf = PerkMovie;
                perk.Conditions.Add(strengthAtLeast6());
            });
            var cyclicLast = plugin.AddPerk("FixtureCyclicPerk2", perk =>
            {
                perk.Level = 10;
                perk.NumRanks = 2;
                perk.Playable = true;
                perk.Conditions.Add(strengthAtLeast6());
            });
            cyclicFirst.NextPerk.FormKey = cyclicLast.FormKey;
            cyclicLast.NextPerk.FormKey = cyclicFirst.FormKey;

            var openFirst = plugin.AddPerk("FixtureOpenPerk1", perk =>
            {
                perk.Level = 5;
                perk.NumRanks = 2;
                perk.Playable = true;
                perk.Conditions.Add(plugin.ActorValueCondition("Perception", CompareOperator.GreaterThanOrEqualTo, 4f));
            });
            var openLast = plugin.AddPerk("FixtureOpenPerk2", perk =>
            {
                perk.Level = 15;
                perk.NumRanks = 2;
                perk.Playable = true;
                perk.Conditions.Add(plugin.ActorValueCondition("Perception", CompareOperator.GreaterThanOrEqualTo, 4f));
            });
            openFirst.NextPerk.FormKey = openLast.FormKey;

            plugin.AddPerk("FixtureHiddenPerk", perk =>
            {
                perk.Playable = true;
                perk.Hidden = true;
                perk.Conditions.Add(plugin.ActorValueCondition("Agility", CompareOperator.GreaterThanOrEqualTo, 3f));
            });
            plugin.AddPerk("FixtureUnplayablePerk", perk =>
                perk.Conditions.Add(plugin.ActorValueCondition("Agility", CompareOperator.GreaterThanOrEqualTo, 3f)));
            plugin.AddPerk("FixtureRaiseStrengthPerk", perk =>
            {
                perk.Playable = true;
                perk.Conditions.Add(plugin.ActorValueCondition("Strength", CompareOperator.LessThan, 10f));
            });
            plugin.AddPerk("FixtureConditionlessPerk", perk => perk.Playable = true);
            plugin.AddPerk("FixtureTwoSpecialPerk", perk =>
            {
                perk.Playable = true;
                perk.Conditions.Add(strengthAtLeast6());
                perk.Conditions.Add(plugin.ActorValueCondition("Perception", CompareOperator.GreaterThanOrEqualTo, 2f));
            });
            plugin.AddPerk("FixtureSharedCellPerk", perk =>
            {
                perk.Level = 20;
                perk.Playable = true;
                perk.Conditions.Add(strengthAtLeast6());
            });
        }

        public const string FirstGrass = "FixtureGrassA";
        public const string SecondGrass = "FixtureGrassB";

        public const string GripAttachPoint = "ap_gun_Grip";
        public const string MuzzleAttachPoint = "ap_gun_Muzzle";
        public const string GripModel = @"weapons	est\grip.nif";

        static void DescribeWeapons(TinyPlugin plugin)
        {
            plugin.AddKeyword(GripAttachPoint);
            plugin.AddKeyword(MuzzleAttachPoint);
            plugin.AddWeaponModification("ModNullMuzzle", mod =>
            {
                mod.AttachPoint.FormKey = plugin.FormKeyOf(MuzzleAttachPoint);
            });
            plugin.AddWeaponModification("ModGrip", mod =>
            {
                mod.Model = new Model { File = GripModel };
                mod.AttachPoint.FormKey = plugin.FormKeyOf(GripAttachPoint);
                mod.AttachParentSlots.Add(new FormLink<IKeywordGetter>(plugin.FormKeyOf(MuzzleAttachPoint)));
                mod.AttachParentSlots.Add(new FormLink<IKeywordGetter>(plugin.FormKeyOf(GripAttachPoint)));
                mod.Includes.Add(new ObjectModInclude { Mod = new FormLink<IObjectModificationGetter>(plugin.FormKeyOf("ModNullMuzzle")), MinimumLevel = 2, Optional = true });
                mod.Properties.Add(new ObjectModIntProperty<Weapon.Property> { Property = Weapon.Property.Speed, Value = 5, Value2 = 6 });
                mod.Properties.Add(new ObjectModFormLinkIntProperty<Weapon.Property> { Property = Weapon.Property.Ammo, Record = new FormLink<IAmmunitionGetter>(plugin.FormKeyOf(GripAttachPoint)), Value = 3 });
            });
            plugin.AddWeapon("TestPistol", @"Weapons\Test\Dummy.nif", weapon =>
            {
                weapon.Keywords = [new FormLink<IKeywordGetter>(plugin.FormKeyOf(MuzzleAttachPoint)), new FormLink<IKeywordGetter>(plugin.FormKeyOf(GripAttachPoint))];
                weapon.ObjectTemplates = [
                    new ObjectTemplate<Weapon.Property> { Name = "Alternative", AddonIndex = -1 },
                    new ObjectTemplate<Weapon.Property>
                    {
                        Name = "Default",
                        AddonIndex = -1,
                        Default = true,
                        Includes = [
                            new ObjectTemplateInclude { Mod = new FormLink<IObjectModificationGetter>(plugin.FormKeyOf("ModGrip")), DontUseAll = true },
                            new ObjectTemplateInclude { Mod = new FormLink<IObjectModificationGetter>(plugin.FormKeyOf("ModNullMuzzle")) },
                        ],
                    },
                ];
            });
            plugin.AddWeapon("TestPistolNoDefault", @"Weapons\Test\Dummy.nif", weapon =>
                weapon.ObjectTemplates = [new ObjectTemplate<Weapon.Property> { Name = "Only", AddonIndex = -1 }]);
            plugin.AddWeapon("TestGrenade", @"Weapons\Grenade.nif");
        }

        static void DescribeGrass(TinyPlugin plugin)
        {
            plugin.AddGrass(FirstGrass, @"Landscape\Grass\Tall.nif", GrassFixture.ConfigureFirst);
            plugin.AddGrass(SecondGrass, @"Landscape\Grass\Short.nif");
            plugin.AddLandscapeTexture("TwoGrassTexture", texture =>
            {
                texture.Grasses.Add(new FormLink<IGrassGetter>(plugin.FormKeyOf(SecondGrass)));
                texture.Grasses.Add(new FormLink<IGrassGetter>(plugin.FormKeyOf(FirstGrass)));
            });
            plugin.AddLandscapeTexture("OneGrassTexture", texture =>
                texture.Grasses.Add(new FormLink<IGrassGetter>(plugin.FormKeyOf(FirstGrass))));
        }

        public const float NoWaterSentinel = float.MaxValue;

        /// <summary>Mode floats with a top byte set (0x41D00000, 0x42780000), which Mutagen's DNAM model drops.</summary>
        public const float FixtureImageSpaceMode = 26f;
        public const float FixtureShortImageSpaceMode = 62f;
        public const float FixtureVignetteRadius = 40f;
        public const float FixtureVignetteStrength = 0.25f;

        /// <summary>PNAM: Use Land Data 0x01, Use Water Data 0x08, Use Sky Cell 0x40.</summary>
        public const int ParentFlagsRaw = 0x01 | 0x08 | 0x40;

        public static readonly float[][] CellCurrents = [[1f, 2f, 3f, 4f], [-5f, 0.5f, 0f, 9f], [7f, 8f, 9f, 10f]];
        public static readonly float[][] PlacedCurrents = [[0.25f, -0.5f, 0.75f, 1f]];

        static void DescribeWater(TinyPlugin plugin)
        {
            plugin.AddWater("FixtureWater", WaterFixture.Configure);
            plugin.AddActivator("WaterActivator", @"Water\Water1024.nif", a => a.WaterType.FormKey = plugin.FormKeyOf("FixtureWater"));
            plugin.AddActivator("DryActivator", @"Furniture\Crate.nif");

            var world = plugin.Worldspace;
            world.Water.FormKey = plugin.FormKeyOf("FixtureWater");
            world.LodWater.FormKey = plugin.FormKeyOf("FixtureWater");
            world.LodWaterHeight = 450f;
            world.LandDefaults = new WorldspaceLandDefaults { DefaultLandHeight = -2048f, DefaultWaterHeight = -10000f };
            world.WaterEnvironmentMap = @"data\Textures\Shared\Cubemaps\fixture.dds";
            world.Parent = new WorldspaceParent
            {
                Worldspace = new FormLink<IWorldspaceGetter>(world.FormKey),
                Flags = (WorldspaceParent.Flag)ParentFlagsRaw,
            };

            plugin.Exterior.Flags |= Cell.Flag.HasWater | Cell.Flag.NoLodWater;
            plugin.Exterior.WaterHeight = 128.5f;
            plugin.Exterior.Water.FormKey = plugin.FormKeyOf("FixtureWater");
            plugin.Exterior.WaterEnvironmentMap = @"data\Textures\Shared\Cubemaps\cell.dds";
            plugin.Interior.WaterHeight = NoWaterSentinel;
            plugin.PersistentCell.WaterHeight = 0f;
        }

        static void DescribeWeather(TinyPlugin plugin) =>
            plugin.AddWeather("FixtureWeather", weather =>
            {
                weather.SkyUpperColor.Day = TinyPlugin.Rgb(10, 20, 30);
                weather.SkyUpperColor.Night = TinyPlugin.Rgb(1, 2, 3);
                weather.FogDistanceDayNear = 100f;
                weather.FogDistanceNightFar = 9000f;
            });

        static void DescribeExteriorRefs(TinyPlugin plugin)
        {
            plugin.PlaceInExterior("PlainRef", "WallStatic", r =>
            {
                r.Position = new P3Float(100f, 200f, 300f);
                r.Rotation = new P3Float(0.1f, 0.2f, 0.3f);
                r.Scale = 2.5f;
            });
            plugin.PlaceInExterior("UnscaledRef", "WallStatic");
            plugin.PlaceInExterior("WaterRef", "WaterActivator");
            plugin.PlaceInExterior("SwappedRef", "WallStatic", r => r.MaterialSwap.FormKey = plugin.FormKeyOf("RedSwap"));
            plugin.PlaceInExterior("MarkerRef", "MarkerStatic");
            plugin.PlaceInExterior("DisabledRef", "WallStatic", r => r.MajorRecordFlagsRaw = TinyPlugin.DisabledFlag);
            plugin.PlaceInExterior("DeletedRef", "WallStatic", _ => { });
            plugin.PlaceBaselessInExterior("BaselessRef");

            plugin.PlaceInExterior("OffRoot", "WallStatic", r => r.MajorRecordFlagsRaw = TinyPlugin.DisabledFlag);
            plugin.PlaceInExterior("OnRoot", "WallStatic");
            plugin.PlaceInExterior("FollowsOffRoot", "WallStatic", r => ParentTo(r, plugin["OffRoot"], opposite: false));
            plugin.PlaceInExterior("InvertsOffRoot", "WallStatic", r => ParentTo(r, plugin["OffRoot"], opposite: true));
            plugin.PlaceInExterior("FollowsOnRoot", "WallStatic", r => ParentTo(r, plugin["OnRoot"], opposite: false));
            plugin.PlaceInExterior("InvertsOnRoot", "WallStatic", r => ParentTo(r, plugin["OnRoot"], opposite: true));
            plugin.PlaceInExterior("InvertsTheInverter", "WallStatic", r => ParentTo(r, plugin["InvertsOffRoot"], opposite: true));
            plugin.PlaceInExterior("OwnDisabledFlagIgnoredUnderParent", "WallStatic", r =>
            {
                r.MajorRecordFlagsRaw = TinyPlugin.DisabledFlag;
                ParentTo(r, plugin["OnRoot"], opposite: false);
            });
            plugin.PlaceInExterior("FollowsPlayer", "WallStatic", r => ParentTo(r, TinyPlugin.PlayerRefFormKey, opposite: false));
            plugin.PlaceInExterior("InvertsPlayer", "WallStatic", r => ParentTo(r, TinyPlugin.PlayerRefFormKey, opposite: true));
            plugin.PlaceInExterior("CycleA", "WallStatic");
            plugin.PlaceInExterior("CycleB", "WallStatic", r => ParentTo(r, plugin["CycleA"], opposite: false));
            ParentTo((PlacedObject)plugin.Exterior.Temporary.Single(p => p.EditorID == "CycleA"), plugin["CycleB"], opposite: false);

            plugin.PlaceInExterior("LampRef", "LampLight", r =>
            {
                r.Radius = 25f;
                r.Lighting = new PlacedObjectLighting
                {
                    Fov90PlusMinus = 5f,
                    Fade1PlusMinus = 0.25f,
                    EndDistanceCap = 4000f,
                    ShadowDepthBias = 0.5f,
                    NearClip = 12f,
                    VolumetricIntensity = 0.75f,
                };
                r.MajorRecordFlagsRaw = 0x200 | 0x10000;
            });
            plugin.PlaceInExterior("PosedRef", "WallStatic", r =>
            {
                r.RagdollData =
                [
                    new RagdollData { BoneId = 0, Unused = new byte[3], Position = new P3Float(1f, 2f, 3f), Rotation = new P3Float(0.5f, 0f, -0.5f) },
                    new RagdollData { BoneId = 7, Unused = new byte[3], Position = new P3Float(-4f, 5f, 6f), Rotation = new P3Float(0f, 1f, 0f) },
                ];
            });
            plugin.PlaceNpcInExterior("ScaledNpc", FormKey.Factory("000ABC:Fallout4.esm"), npc =>
            {
                npc.Scale = 1.25f;
                npc.Position = new P3Float(7f, 8f, 9f);
            });
            plugin.PlaceInPersistentCell("WorldPersistentRef", "WallStatic");
        }

        static LayerHeader LayerOf(TinyPlugin plugin, Quadrant quadrant, ushort layerNumber)
        {
            var header = new LayerHeader { Quadrant = quadrant, LayerNumber = layerNumber };
            header.Texture.FormKey = plugin.FormKeyOf("GrassTexture");
            return header;
        }

        static void DescribeLandscape(TinyPlugin plugin) =>
            plugin.AddLandscapeToExterior(landscape =>
            {
                landscape.VertexHeightMap = new LandscapeVertexHeightMap { Offset = 42.5f };
                landscape.VertexHeightMap.HeightMap[3, 2] = 5;
                landscape.VertexHeightMap.HeightMap[32, 32] = 249;
                landscape.VertexColors = new Array2d<P3UInt8>(new P3UInt8[33, 33]);
                landscape.VertexColors[1, 0] = new P3UInt8(10, 20, 30);
                landscape.VertexColors[0, 1] = new P3UInt8(40, 50, 60);
                landscape.Layers.Add(new BaseLayer { Header = LayerOf(plugin, Quadrant.BottomLeft, ushort.MaxValue) });
                landscape.Layers.Add(new AlphaLayer { Header = LayerOf(plugin, Quadrant.TopRight, 1), AlphaLayerData = new byte[] { 1, 2, 3, 4, 5, 6 } });
            });

        static void DescribeInterior(TinyPlugin plugin)
        {
            plugin.Interior.Lighting = new CellLighting
            {
                AmbientColor = TinyPlugin.Rgb(10, 20, 30),
                DirectionalColor = TinyPlugin.Rgb(200, 100, 50),
                FogNear = 64f,
                FogFar = 2048f,
                DirectionalRotationXY = 45,
                DirectionalRotationZ = -30,
            };
            plugin.PlaceInExterior("ExteriorDoor", "DoorBase");
            plugin.PlaceInInterior("InteriorDoor", "DoorBase", r =>
                r.TeleportDestination = new TeleportDestination
                {
                    Door = new FormLink<IPlacedObjectGetter>(plugin.FormKeyOf("ExteriorDoor")),
                    Position = new P3Float(10f, 20f, 30f),
                    Rotation = new P3Float(0f, 0f, 1.5f),
                });
            var exteriorDoor = (PlacedObject)plugin.Exterior.Temporary.Single(p => p.EditorID == "ExteriorDoor");
            exteriorDoor.TeleportDestination = new TeleportDestination
            {
                Door = new FormLink<IPlacedObjectGetter>(plugin.FormKeyOf("InteriorDoor")),
                Position = new P3Float(-1f, -2f, -3f),
                Rotation = new P3Float(0f, 0f, 0f),
            };
            plugin.PlaceInInterior("InteriorPlain", "WallStatic", r => r.Position = new P3Float(1f, 1f, 1f));
        }

        static void ParentTo(PlacedObject child, string parentFormKey, bool opposite)
        {
            child.EnableParent = new EnableParent
            {
                Reference = new FormLink<ILinkedReferenceGetter>(Mutagen.Bethesda.Plugins.FormKey.Factory(parentFormKey)),
                Flags = opposite ? EnableParent.Flag.SetEnableStateToOppositeOfParent : 0,
            };
        }
    }

    internal sealed class TempDirectory : IDisposable
    {
        public string Path { get; } = System.IO.Path.Combine(System.IO.Path.GetTempPath(), "openfo4-test-" + Guid.NewGuid().ToString("N"));

        public TempDirectory() => Directory.CreateDirectory(Path);

        public void Dispose() => Directory.Delete(Path, recursive: true);
    }
}
