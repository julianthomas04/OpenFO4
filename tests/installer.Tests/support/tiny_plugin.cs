using System.Drawing;
using Mutagen.Bethesda;
using Mutagen.Bethesda.Fallout4;
using Mutagen.Bethesda.Plugins;
using Mutagen.Bethesda.Strings;
using Noggog;

namespace InstallerTests
{
    /// <summary>
    /// A tiny Fallout 4 plugin built with Mutagen's own in-memory record objects and written as a real
    /// binary ESM, so the loader reads it through the same overlay it uses on Fallout4.esm.
    /// Names are the editor ids; <see cref="FormKeys"/> maps them to the form key strings the loader stores.
    /// </summary>
    internal sealed class TinyPlugin
    {
        public const string PluginFileName = "Fixture.esm";
        public const string ExteriorCellEditorId = "FixtureExterior";
        public const string InteriorCellEditorId = "FixtureInterior";
        public const string PersistentCellEditorId = "FixtureWorldTopCell";
        public const int ExteriorGridX = 1;
        public const int ExteriorGridY = -2;
        public const int DisabledFlag = 0x800;
        public const int MarkerFlag = 0x800000;
        public const string PlayerRefFormKey = "000014:Fallout4.esm";

        readonly Fallout4Mod _mod = new(ModKey.FromNameAndExtension(PluginFileName), Fallout4Release.Fallout4);
        readonly Dictionary<string, string> _formKeys = new();
        readonly List<(FunctionConditionData Data, string ActorValueEditorId)> _pendingRecordParameters = new();
        readonly Cell _exterior;
        readonly Cell _interior;
        readonly Cell _persistentCell;

        public IReadOnlyDictionary<string, string> FormKeys => _formKeys;

        public string this[string editorId] => _formKeys[editorId];

        public TinyPlugin()
        {
            _exterior = new Cell(_mod) { EditorID = ExteriorCellEditorId, Grid = new CellGrid { Point = new P2Int(ExteriorGridX, ExteriorGridY) } };
            _persistentCell = new Cell(_mod) { EditorID = PersistentCellEditorId };
            _interior = new Cell(_mod) { EditorID = InteriorCellEditorId, Flags = Cell.Flag.IsInteriorCell };
            Register(_exterior.EditorID, _exterior.FormKey);
            Register(_persistentCell.EditorID, _persistentCell.FormKey);
            Register(_interior.EditorID, _interior.FormKey);
        }

        public Static AddStatic(string editorId, string model, int majorFlags = 0)
        {
            var record = _mod.Statics.AddNew(editorId);
            record.Model = new Model { File = model };
            record.ObjectBounds = new ObjectBounds { First = new P3Int16(-10, -20, -30), Second = new P3Int16(10, 20, 30) };
            record.MajorRecordFlagsRaw = majorFlags;
            Register(editorId, record.FormKey);
            return record;
        }

        public Door AddDoor(string editorId, string model)
        {
            var record = _mod.Doors.AddNew(editorId);
            record.Model = new Model { File = model };
            Register(editorId, record.FormKey);
            return record;
        }

        public Light AddLight(string editorId, Action<Light> configure)
        {
            var record = _mod.Lights.AddNew(editorId);
            configure(record);
            Register(editorId, record.FormKey);
            return record;
        }

        public Weather AddWeather(string editorId, Action<Weather> configure)
        {
            var record = _mod.Weather.AddNew(editorId);
            configure(record);
            Register(editorId, record.FormKey);
            return record;
        }

        public ImageSpace AddImageSpace(string editorId, Action<ImageSpace> configure)
        {
            var record = _mod.ImageSpaces.AddNew(editorId);
            configure(record);
            Register(editorId, record.FormKey);
            return record;
        }

        public Keyword AddKeyword(string editorId)
        {
            var record = _mod.Keywords.AddNew(editorId);
            Register(editorId, record.FormKey);
            return record;
        }

        public Weapon AddWeapon(string editorId, string? model, Action<Weapon>? configure = null)
        {
            var record = _mod.Weapons.AddNew(editorId);
            if (model != null) record.Model = new Model { File = model };
            configure?.Invoke(record);
            Register(editorId, record.FormKey);
            return record;
        }

        public WeaponModification AddWeaponModification(string editorId, Action<WeaponModification>? configure = null)
        {
            var record = new WeaponModification(_mod) { EditorID = editorId };
            configure?.Invoke(record);
            _mod.ObjectModifications.Add(record);
            Register(editorId, record.FormKey);
            return record;
        }

        public Water AddWater(string editorId, Action<Water>? configure = null)
        {
            var record = _mod.Waters.AddNew(editorId);
            configure?.Invoke(record);
            Register(editorId, record.FormKey);
            return record;
        }

        public MaterialType AddMaterialType(string editorId, Action<MaterialType>? configure = null)
        {
            var record = _mod.MaterialTypes.AddNew(editorId);
            configure?.Invoke(record);
            Register(editorId, record.FormKey);
            return record;
        }

        public Mutagen.Bethesda.Fallout4.Activator AddActivator(string editorId, string model, Action<Mutagen.Bethesda.Fallout4.Activator>? configure = null)
        {
            var record = _mod.Activators.AddNew(editorId);
            record.Model = new Model { File = model };
            configure?.Invoke(record);
            Register(editorId, record.FormKey);
            return record;
        }

        public ActorValueInformation AddActorValue(string editorId, string? name = null, string? description = null)
        {
            var record = _mod.ActorValueInformation.AddNew(editorId);
            if (name is not null) record.Name = new TranslatedString(Language.English, name);
            if (description is not null) record.Description = new TranslatedString(Language.English, description);
            Register(editorId, record.FormKey);
            return record;
        }

        public AddonNode AddAddonNode(string editorId, string model, Action<AddonNode>? configure = null)
        {
            var record = _mod.AddonNodes.AddNew(editorId);
            record.Model = new Model { File = model };
            configure?.Invoke(record);
            Register(editorId, record.FormKey);
            return record;
        }

        public Perk AddPerk(string editorId, Action<Perk> configure)
        {
            var record = _mod.Perks.AddNew(editorId);
            configure(record);
            Register(editorId, record.FormKey);
            return record;
        }

        /// <summary>
        /// A perk condition comparing the base value of an actor value with a constant. Mutagen writes a generic function
        /// condition's record parameter from its raw number, which holds the master index, so it is set when the plugin is written.
        /// </summary>
        public Condition ActorValueCondition(string actorValueEditorId, CompareOperator compare, float value)
        {
            var data = new FunctionConditionData { Function = Condition.Function.GetBaseValue };
            _pendingRecordParameters.Add((data, actorValueEditorId));
            return new ConditionFloat { CompareOperator = compare, ComparisonValue = value, Data = data };
        }

        void ResolveRecordParameters()
        {
            uint ownIndex = (uint)_mod.EnumerateFormLinks().Select(link => link.FormKey.ModKey).Where(modKey => modKey != _mod.ModKey).Distinct().Count();
            foreach (var (data, editorId) in _pendingRecordParameters)
                data.ParameterOneNumber = (int)((ownIndex << 24) | FormKeyOf(editorId).ID);
        }

        public Climate AddClimate(string editorId)
        {
            var record = _mod.Climates.AddNew(editorId);
            Register(editorId, record.FormKey);
            return record;
        }

        public LandscapeTexture AddLandscapeTexture(string editorId, Action<LandscapeTexture>? configure = null)
        {
            var record = _mod.LandscapeTextures.AddNew(editorId);
            configure?.Invoke(record);
            Register(editorId, record.FormKey);
            return record;
        }

        public Grass AddGrass(string editorId, string model, Action<Grass>? configure = null)
        {
            var record = _mod.Grasses.AddNew(editorId);
            record.Model = new Model { File = model };
            configure?.Invoke(record);
            Register(editorId, record.FormKey);
            return record;
        }

        public MaterialSwap AddMaterialSwap(string editorId)
        {
            var record = _mod.MaterialSwaps.AddNew(editorId);
            Register(editorId, record.FormKey);
            return record;
        }

        public PlacedObject PlaceInExterior(string editorId, string baseEditorId, Action<PlacedObject>? configure = null) =>
            Place(_exterior.Temporary, editorId, baseEditorId, configure);

        public PlacedObject PlaceInPersistentCell(string editorId, string baseEditorId, Action<PlacedObject>? configure = null) =>
            Place(_persistentCell.Persistent, editorId, baseEditorId, configure);

        public PlacedObject PlaceInInterior(string editorId, string baseEditorId, Action<PlacedObject>? configure = null) =>
            Place(_interior.Temporary, editorId, baseEditorId, configure);

        public PlacedNpc PlaceNpcInExterior(string editorId, FormKey npcBase, Action<PlacedNpc>? configure = null)
        {
            var record = new PlacedNpc(_mod) { EditorID = editorId };
            record.Base.FormKey = npcBase;
            configure?.Invoke(record);
            _exterior.Temporary.Add(record);
            Register(editorId, record.FormKey);
            return record;
        }

        /// <summary>A ref whose form key exists but which carries no base object (the loader must skip it).</summary>
        public PlacedObject PlaceBaselessInExterior(string editorId)
        {
            var record = new PlacedObject(_mod) { EditorID = editorId };
            _exterior.Temporary.Add(record);
            Register(editorId, record.FormKey);
            return record;
        }

        public Landscape AddLandscapeToExterior(Action<Landscape> configure)
        {
            var landscape = new Landscape(_mod);
            configure(landscape);
            _exterior.Landscape = landscape;
            Register("FixtureLandscape", landscape.FormKey);
            return landscape;
        }

        /// <summary>A NAVM in the exterior cell, declared to belong to the given grid cell of the fixture worldspace.</summary>
        public NavMeshBuilder AddExteriorNavMesh(string name, int gridX = ExteriorGridX, int gridY = ExteriorGridY, FormKey? declaredWorldspace = null) =>
            AttachNavMesh(name, _exterior, NavMeshBuilder.Exterior(_mod, declaredWorldspace ?? Worldspace.FormKey, gridX, gridY));

        /// <summary>A mesh declaring an exterior parent but sitting in the interior cell, which has no grid position.</summary>
        public NavMeshBuilder AddExteriorNavMeshInInteriorCell(string name) =>
            AttachNavMesh(name, _interior, NavMeshBuilder.Exterior(_mod, Worldspace.FormKey, ExteriorGridX, ExteriorGridY));

        public NavMeshBuilder AddInteriorNavMesh(string name, FormKey? parentCell = null) =>
            AttachNavMesh(name, _interior, NavMeshBuilder.Interior(_mod, parentCell ?? _interior.FormKey));

        NavMeshBuilder AttachNavMesh(string name, Cell cell, NavMeshBuilder builder)
        {
            cell.NavigationMeshes.Add(builder.Mesh);
            Register(name, builder.Mesh.FormKey);
            return builder;
        }

        public MusicType AddMusicType(string editorId, Action<MusicType> configure)
        {
            var record = _mod.MusicTypes.AddNew(editorId);
            configure(record);
            Register(editorId, record.FormKey);
            return record;
        }

        public MusicTrack AddMusicTrack(string editorId, Action<MusicTrack> configure)
        {
            var record = _mod.MusicTracks.AddNew(editorId);
            configure(record);
            Register(editorId, record.FormKey);
            return record;
        }

        public SoundDescriptor AddSoundDescriptor(string editorId, Action<SoundDescriptor> configure)
        {
            var record = _mod.SoundDescriptors.AddNew(editorId);
            configure(record);
            Register(editorId, record.FormKey);
            return record;
        }

        public SoundOutputModel AddSoundOutputModel(string editorId, Action<SoundOutputModel> configure)
        {
            var record = _mod.SoundOutputModels.AddNew(editorId);
            configure(record);
            Register(editorId, record.FormKey);
            return record;
        }

        public FootstepSet AddFootstepSet(string editorId, Action<FootstepSet> configure)
        {
            var record = _mod.FootstepSets.AddNew(editorId);
            configure(record);
            Register(editorId, record.FormKey);
            return record;
        }

        public Footstep AddFootstep(string editorId, Action<Footstep> configure)
        {
            var record = _mod.Footsteps.AddNew(editorId);
            configure(record);
            Register(editorId, record.FormKey);
            return record;
        }

        public ImpactDataSet AddImpactDataSet(string editorId, Action<ImpactDataSet> configure)
        {
            var record = _mod.ImpactDataSets.AddNew(editorId);
            configure(record);
            Register(editorId, record.FormKey);
            return record;
        }

        public Impact AddImpact(string editorId, Action<Impact> configure)
        {
            var record = _mod.Impacts.AddNew(editorId);
            configure(record);
            Register(editorId, record.FormKey);
            return record;
        }

        public DefaultObjectManager AddDefaultObjectManager(string editorId, Action<DefaultObjectManager> configure)
        {
            var record = _mod.DefaultObjectManagers.AddNew(editorId);
            configure(record);
            Register(editorId, record.FormKey);
            return record;
        }

        public Mutagen.Bethesda.Fallout4.Region AddRegion(string editorId, Action<Mutagen.Bethesda.Fallout4.Region> configure)
        {
            var record = _mod.Regions.AddNew(editorId);
            configure(record);
            Register(editorId, record.FormKey);
            return record;
        }

        public SoundCategory AddSoundCategory(string editorId, Action<SoundCategory> configure)
        {
            var record = _mod.SoundCategories.AddNew(editorId);
            configure(record);
            Register(editorId, record.FormKey);
            return record;
        }

        public Location AddLocation(string editorId, Action<Location> configure)
        {
            var record = _mod.Locations.AddNew(editorId);
            configure(record);
            Register(editorId, record.FormKey);
            return record;
        }

        public Global AddGlobal(string editorId, Func<IFallout4Mod, Global> create)
        {
            var record = create(_mod);
            record.EditorID = editorId;
            _mod.Globals.Add(record);
            Register(editorId, record.FormKey);
            return record;
        }

        public Cell Exterior => _exterior;

        public Cell Interior => _interior;

        public Cell PersistentCell => _persistentCell;

        public FormKey FormKeyOf(string editorId) => FormKey.Factory(_formKeys[editorId]);

        public string Write(string directory)
        {
            AssembleWorldspace();
            AssembleInteriorBlock();
            ResolveRecordParameters();
            string path = Path.Combine(directory, PluginFileName);
            _mod.WriteToBinary(path);
            return path;
        }

        Worldspace? _worldspace;

        public Worldspace Worldspace => _worldspace ?? throw new InvalidOperationException("Call AddWorldspace first");

        public Worldspace AddWorldspace(string editorId)
        {
            _worldspace = _mod.Worldspaces.AddNew(editorId);
            Register(editorId, _worldspace.FormKey);
            return _worldspace;
        }

        void AssembleWorldspace()
        {
            if (_worldspace is null) return;
            _worldspace.TopCell = _persistentCell;
            var subBlock = new WorldspaceSubBlock { BlockNumberX = 0, BlockNumberY = -1 };
            subBlock.Items.Add(_exterior);
            var block = new WorldspaceBlock { BlockNumberX = 0, BlockNumberY = -1 };
            block.Items.Add(subBlock);
            _worldspace.SubCells.Add(block);
        }

        void AssembleInteriorBlock()
        {
            var subBlock = new CellSubBlock { BlockNumber = 0 };
            subBlock.Cells.Add(_interior);
            var block = new CellBlock { BlockNumber = 0 };
            block.SubBlocks.Add(subBlock);
            _mod.Cells.Records.Add(block);
        }

        PlacedObject Place(ExtendedList<IPlaced> list, string editorId, string baseEditorId, Action<PlacedObject>? configure)
        {
            var record = new PlacedObject(_mod) { EditorID = editorId };
            record.Base.FormKey = FormKeyOf(baseEditorId);
            configure?.Invoke(record);
            list.Add(record);
            Register(editorId, record.FormKey);
            return record;
        }

        void Register(string? editorId, FormKey formKey) => _formKeys[editorId ?? throw new ArgumentNullException(nameof(editorId))] = formKey.ToString();

        public static Color Rgb(int r, int g, int b) => Color.FromArgb(255, r, g, b);
    }
}
