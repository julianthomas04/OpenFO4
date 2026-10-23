using System.Globalization;
using System.Text.RegularExpressions;

namespace AssetConverter
{
    /// <summary>A clip taken from the first frame of the symbol a movie exports under a class name.</summary>
    public sealed record ClassClipSource(string ClipId, string Movie, string ClassName);

    /// <summary>How a symbol is found in its movie: by names the movie's own structure states, never by character id, depth or frame number.</summary>
    public abstract record SymbolRef
    {
        /// <summary>The symbol exported under exactly this class name.</summary>
        public sealed record ByClass(string ClassName) : SymbolRef;

        /// <summary>The only symbol whose class name starts with this prefix, for classes with an auto-generated numeric suffix.</summary>
        public sealed record ByClassPrefix(string Prefix) : SymbolRef;

        /// <summary>The only character placed under this instance name inside another symbol.</summary>
        public sealed record NamedChild(SymbolRef Parent, string InstanceName) : SymbolRef;

        /// <summary>The only object without an instance name inside another symbol.</summary>
        public sealed record OnlyUnnamedChild(SymbolRef Parent) : SymbolRef;

        /// <summary>The only unnamed object that appears in the frame a label names inside another symbol.</summary>
        public sealed record UnnamedAtLabel(SymbolRef Parent, string Label) : SymbolRef;

        /// <summary>
        /// The unnamed shape or sprite, placed once in another symbol, that lies nearest to the object named <paramref name="SiblingName"/>:
        /// the glyph an entry's label sits beside.
        /// </summary>
        public sealed record UnnamedBeside(SymbolRef Parent, string SiblingName) : SymbolRef;

        /// <summary>The sprite that actually animates inside a single-frame wrapper symbol.</summary>
        public sealed record AnimatedContentOf(SymbolRef Wrapper) : SymbolRef;
    }

    /// <summary>Which timeline frames of a symbol make up a clip.</summary>
    public abstract record FrameSelection
    {
        public static readonly FrameSelection EveryFrame = new All();

        public static readonly FrameSelection FirstFrame = new Only(1);

        /// <summary>Every frame.</summary>
        public sealed record All : FrameSelection;

        /// <summary>One frame, 1-based.</summary>
        public sealed record Only(int Frame) : FrameSelection;

        /// <summary>The frame a label names.</summary>
        public sealed record Labelled(string Label) : FrameSelection;

        /// <summary>
        /// Every frame before the frames that show a text field instead of art (the game draws those as text); a text frame before art
        /// is an error.
        /// </summary>
        public sealed record UntilTrailingText : FrameSelection;
    }

    /// <summary>A clip made of the <paramref name="Frames"/> of a symbol found through a <see cref="SymbolRef"/>.</summary>
    public sealed record NamedSymbolClipSource(string ClipId, string Movie, SymbolRef Symbol, FrameSelection Frames, ArtColour Colour = ArtColour.Mask);

    /// <summary>A damage type: the kind its clips are named by and the value the game hands the item cards for it.</summary>
    public sealed record DamageType(string Kind, int Value);

    /// <summary>
    /// A paper doll piece: the first frame of the symbol placed inside the doll container, found by its instance name or, for the
    /// pieces that replace each other, as the one unnamed placement of the frame with the given label.
    /// </summary>
    public sealed record DollPieceSource(string ClipId, string Movie, string ContainerClass, string? InstanceName, string? FrameLabel);

    /// <summary>A clip made of every timeline frame of a whole movie; with <paramref name="TightBounds"/> its bounds cover everything the frames show instead of the stage rectangle.</summary>
    public sealed record MovieClipSource(string ClipId, string Movie, bool TightBounds = false);

    /// <summary>
    /// Which game movies and symbols become which Pip-Boy art clips. Symbols are named by class, instance path or frame label; the clip
    /// ids are the contract with the engine's <c>pipboy_art.rs</c>. Counts and enumerations the movies state (the map marker classes, the
    /// condition figures) are read from them at conversion time.
    /// </summary>
    public static partial class PipboyArtCatalog
    {
        public const string InventoryPage = "Pipboy_InvPage.swf";
        public const string StatsPage = "Pipboy_StatsPage.swf";
        public const string DataPage = "Pipboy_DataPage.swf";
        public const string MapPage = "Pipboy_MapPage.swf";
        public const string Shell = "PipboyMenu.swf";
        public const string LevelUpMenu = "LevelUpMenu.swf";
        public const string LoadingScreen = "LoadingMenu.swf";

        public const string PerkCardClassPrefix = "PerkClip";
        public const string PerkMovieFolder = @"Components\VaultBoys\Perks\";
        public const string PerkMovieNamePrefix = "PerkClip_";

        public static readonly IReadOnlyList<string> VaultBoyFolders =
        [
            @"Components\VaultBoys\",
            @"Components\Quest Vault Boys\",
            @"Components\Faction Vault boys\",
            @"Components\Magazine perks\",
        ];

        const string InventoryListEntryClass = "InvListEntry";
        const string WorkshopCardClass = "Workshop_ItemCard";
        const string BottomBarClassPrefix = "PipboyMenu_fla.BottomBar_Info_";
        const string ComparisonClassPrefix = "Pipboy_InvPage_fla.ItemCard_Comparison_";
        const string RankStarClass = "PerkRankStar";
        const string RaiderMarkerClass = "RaiderSettlementMarker";

        static SymbolRef.ByClass Class(string name) => new SymbolRef.ByClass(name);

        static SymbolRef.ByClassPrefix ClassPrefix(string prefix) => new SymbolRef.ByClassPrefix(prefix);

        static SymbolRef.NamedChild Child(SymbolRef parent, string instanceName) => new SymbolRef.NamedChild(parent, instanceName);

        static NamedSymbolClipSource Icon(string clipId, string movie, SymbolRef symbol, FrameSelection? frames = null) =>
            new(clipId, movie, symbol, frames ?? FrameSelection.FirstFrame);

        /// <summary>The pictograms: each is the glyph a movie's own symbol shows, found through the class and instance names around it.</summary>
        public static IReadOnlyList<NamedSymbolClipSource> Icons { get; } =
        [
            Icon("icon/weapon", Shell, Child(Class("BB_DMGDRWidget"), "Icon_mc"), new FrameSelection.Labelled("Weapon")),
            Icon("icon/armor", Shell, Child(Class("BB_DMGDRWidget"), "Icon_mc"), new FrameSelection.Labelled("Armor")),
            Icon("icon/weight", Shell, new SymbolRef.UnnamedBeside(ClassPrefix(BottomBarClassPrefix), "Weight_tf")),
            Icon("icon/caps", Shell, new SymbolRef.UnnamedBeside(ClassPrefix(BottomBarClassPrefix), "Caps_tf")),
            Icon("icon/favourite", InventoryPage, Child(Class(InventoryListEntryClass), "FavIcon_mc")),
            Icon("icon/legendary", InventoryPage, Child(Class(InventoryListEntryClass), "LegendaryIcon_mc")),
            Icon("icon/search_tag", InventoryPage, Child(Class(InventoryListEntryClass), "SearchIcon_mc")),
            Icon("icon/star_empty", StatsPage, Class(RankStarClass), new FrameSelection.Labelled("Empty")),
            Icon("icon/star_filled", StatsPage, Class(RankStarClass), new FrameSelection.Labelled("Full")),
            Icon("icon/star_available", StatsPage, Class(RankStarClass), new FrameSelection.Labelled("Available")),
            Icon("icon/population", DataPage, new SymbolRef.UnnamedBeside(Class(WorkshopCardClass), "PeopleEntry_mc")),
            Icon("icon/food", DataPage, new SymbolRef.UnnamedBeside(Class(WorkshopCardClass), "FoodEntry_mc")),
            Icon("icon/water", DataPage, new SymbolRef.UnnamedBeside(Class(WorkshopCardClass), "WaterEntry_mc")),
            Icon("icon/power", DataPage, new SymbolRef.UnnamedBeside(Class(WorkshopCardClass), "PowerEntry_mc")),
            Icon("icon/defense", DataPage, new SymbolRef.UnnamedBeside(Class(WorkshopCardClass), "SafetyEntry_mc")),
            Icon("icon/beds", DataPage, new SymbolRef.UnnamedBeside(Class(WorkshopCardClass), "BedsEntry_mc")),
            Icon("icon/happiness", DataPage, new SymbolRef.UnnamedBeside(Class(WorkshopCardClass), "HappyEntry_mc")),
            Icon("icon/alert", DataPage, Child(Class("WorkshopsListEntry"), "AlertIcon_mc")),
            Icon("icon/raider_skull_discovered", DataPage, new SymbolRef.UnnamedAtLabel(Class(RaiderMarkerClass), "Discovered")),
            Icon("icon/raider_skull_undiscovered", DataPage, new SymbolRef.UnnamedAtLabel(Class(RaiderMarkerClass), "Undiscovered")),
            Icon("icon/timer", InventoryPage, Child(Class("Components.ItemCard_TimedEntry"), "TimerIcon_mc")),
            Icon("icon/quest_completed", DataPage, new SymbolRef.UnnamedAtLabel(ClassPrefix("Pipboy_DataPage_fla.EquippedIcon_"), "complete")),
            Icon("icon/comparison_better", InventoryPage, new SymbolRef.UnnamedAtLabel(ClassPrefix(ComparisonClassPrefix), "Good")),
            Icon("icon/comparison_worse", InventoryPage, new SymbolRef.UnnamedAtLabel(ClassPrefix(ComparisonClassPrefix), "Bad")),
            Icon("icon/player_arrow", MapPage, new SymbolRef.OnlyUnnamedChild(Class("PlayerLocMarker"))),
            Icon("icon/custom_marker", MapPage, new SymbolRef.UnnamedAtLabel(ClassPrefix("Pipboy_MapPage_fla.PlayerSetMarker_Base_"), "Level")),
        ];

        public const string DollContainerClass = "PaperDoll";

        public static IReadOnlyList<DollPieceSource> DollPieces { get; } =
        [
            new("doll/body", InventoryPage, DollContainerClass, null, "Underwear"),
            new("doll/clothes_1", InventoryPage, DollContainerClass, null, "GenericClothes"),
            new("doll/clothes_2", InventoryPage, DollContainerClass, null, "VaultSuit"),
            new("doll/torso", InventoryPage, DollContainerClass, "Torso_mc", null),
            new("doll/right_arm", InventoryPage, DollContainerClass, "RightArm_mc", null),
            new("doll/left_arm", InventoryPage, DollContainerClass, "LeftArm_mc", null),
            new("doll/right_leg", InventoryPage, DollContainerClass, "RightLeg_mc", null),
            new("doll/left_leg", InventoryPage, DollContainerClass, "LeftLeg_mc", null),
            new("doll/goggles", InventoryPage, DollContainerClass, "Goggles_mc", null),
            new("doll/helmet", InventoryPage, DollContainerClass, "Helmet_mc", null),
            new("doll/gas_mask", InventoryPage, DollContainerClass, "GasMask_mc", null),
        ];

        public const string DamageIconClassPrefix = "Pipboy_InvPage_fla.DamageIcons_";
        public const string HotkeyIconClassPrefix = "Pipboy_InvPage_fla.HotkeyIcons_";
        public const string AmmoEntryClass = "ItemCard_AmmoEntry";
        public const string QuestDefaultBoyClass = "Quest_DefaultBoy";
        public const string QuestDefaultBoyClipId = "quest/default_boy";
        public const string HotkeyClipId = "icon/hotkey";

        /// <summary>The script constant that counts the damage types; the damage icon symbol shows a type's damage glyph that many frames after its resistance glyph.</summary>
        public const string DamageTypeCountConstant = "Shared.GlobalFunc.NUM_DAMAGE_TYPES";

        /// <summary>
        /// STAND-IN, native only: the damage type values the game's native code hands the item cards, and the kind each is named by. The
        /// movie scripts state how many types there are (<see cref="DamageTypeCountConstant"/>) and show a type's glyph on the frame of its
        /// value, but nowhere which value is which kind; <see cref="RequireMatchingTypeCount"/> checks this list against that count. The
        /// damage icon symbol shows a type's resistance glyph on the frame of its value.
        /// </summary>
        public static IReadOnlyList<DamageType> NativeDamageTypes { get; } =
        [
            new("physical", 1),
            new("poison", 2),
            new("fire", 3),
            new("energy", 4),
            new("cryo", 5),
            new("radiation", 6),
        ];

        /// <summary>Throws unless the stand-in list holds exactly the values 1..count the scripts state.</summary>
        public static void RequireMatchingTypeCount(int scriptedCount)
        {
            if (!NativeDamageTypes.Select(type => type.Value).Order().SequenceEqual(Enumerable.Range(1, scriptedCount)))
                throw new PipboyArtException(
                    $"The scripts state {scriptedCount} damage types, but the native damage type stand-in holds {NativeDamageTypes.Count} (values {string.Join(", ", NativeDamageTypes.Select(t => t.Value))}); it needs the values 1..{scriptedCount}.");
        }

        public static string DamageResistClipId(DamageType type) => $"icon/damage_{type.Kind}";

        public static string DamageDealtClipId(DamageType type) => $"icon/damage_dealt_{type.Kind}";

        /// <summary>The resistance and damage glyph clips of every damage type, given the damage type count the scripts state.</summary>
        public static IReadOnlyList<NamedSymbolClipSource> DamageIcons(int damageTypeCount) =>
        [
            .. NativeDamageTypes.SelectMany(type => new[]
            {
                new NamedSymbolClipSource(DamageResistClipId(type), InventoryPage, ClassPrefix(DamageIconClassPrefix), new FrameSelection.Only(type.Value)),
                new NamedSymbolClipSource(DamageDealtClipId(type), InventoryPage, ClassPrefix(DamageIconClassPrefix), new FrameSelection.Only(type.Value + damageTypeCount)),
            }),
        ];

        public static IReadOnlyList<NamedSymbolClipSource> NamedSymbols { get; } =
        [
            Icon("icon/ammo", InventoryPage, new SymbolRef.UnnamedBeside(Class(AmmoEntryClass), "Label_tf")),
            new(HotkeyClipId, InventoryPage, ClassPrefix(HotkeyIconClassPrefix), new FrameSelection.UntilTrailingText()),
            new(QuestDefaultBoyClipId, DataPage, new SymbolRef.AnimatedContentOf(Class(QuestDefaultBoyClass)), FrameSelection.EveryFrame),
        ];

        public const string PerkChartMenuClass = "LevelUpMenu";

        /// <summary>
        /// Symbols the perk chart draws besides its cards, in their own colours: the rank and SPECIAL stars, and the sprite the chart
        /// places without a name in its perk holder.
        /// </summary>
        public static IReadOnlyList<NamedSymbolClipSource> PerkChartSymbols { get; } =
        [
            new("perkchart/rank_star", LevelUpMenu, Class("PerkRankStar"), FrameSelection.EveryFrame, ArtColour.Full),
            new("perkchart/special_star", LevelUpMenu, Class("SPECIALStar"), FrameSelection.EveryFrame, ArtColour.Full),
            new("perkchart/perks_holder_depth_1", LevelUpMenu,
                new SymbolRef.OnlyUnnamedChild(Child(Child(Class(PerkChartMenuClass), "GridView_mc"), "PerksHolder_mc")), FrameSelection.EveryFrame, ArtColour.Full),
        ];

        /// <summary>
        /// Glyphs of the pages' prompts, lists and map: the quantity slider's arrow (the right arrow is the same symbol turned round), the
        /// favourites cross centre, the list scroll arrow (scrolling down is the same symbol turned round), the map's quest, door and
        /// power armour markers and the animated Vault-Tec logo of the loading screen (the sprite inside the logo's instance; the engine
        /// takes its placement from the loading screen's layout document).
        /// </summary>
        public static IReadOnlyList<NamedSymbolClipSource> InterfaceSymbols { get; } =
        [
            new("inv/slider_arrow", InventoryPage, Child(Child(Class("Shared.QuantityMenuNEW"), "Slider_mc"), "LeftArrow_mc"), FrameSelection.EveryFrame),
            new("inv/favourites_centre", InventoryPage, new SymbolRef.OnlyUnnamedChild(Class("FavoritesCross")), FrameSelection.EveryFrame),
            new("list/scroll_arrow", InventoryPage, Child(Class("InvList"), "ScrollUp"), FrameSelection.EveryFrame),
            new("map/quest_marker", MapPage, Class("QuestMarker"), FrameSelection.EveryFrame),
            new("map/quest_marker_door", MapPage, Class("QuestMarkerDoor"), FrameSelection.EveryFrame),
            new("map/door_marker", MapPage, Class("DoorMarker"), FrameSelection.EveryFrame),
            new("map/power_armor_marker", MapPage, Class("PowerArmorLocMarker"), FrameSelection.EveryFrame),
            new(LoadingIconClipId, LoadingScreen, new SymbolRef.AnimatedContentOf(Child(Class(LoadingScreenMenuClass), "VaultTecLogo_mc")), FrameSelection.EveryFrame),
        ];

        public const string LoadingIconClipId = "loading/vault_tec_logo";
        public const string LoadingScreenMenuClass = "LoadingMenu";

        /// <summary>The script member that lists the map marker classes; a marker's type is its index in the list.</summary>
        public const string MarkerListMember = "MapHolder.MarkerTypesA";

        public static string MarkerClipId(int markerType) => string.Create(CultureInfo.InvariantCulture, $"marker/{markerType}");

        /// <summary>The marker clips of a class list: type <c>n</c> is drawn by the symbol of the list's <c>n</c>th class.</summary>
        public static IReadOnlyList<ClassClipSource> MarkerSources(IReadOnlyList<string> classNames) =>
            [.. classNames.Select((className, type) => new ClassClipSource(MarkerClipId(type), MapPage, className))];

        public static readonly IReadOnlyList<string> SpecialNames =
            ["Strength", "Perception", "Endurance", "Charisma", "Intelligence", "Agility", "Luck"];

        public static IReadOnlyList<MovieClipSource> SpecialMovies { get; } =
            SpecialNames.Select(name => new MovieClipSource($"special/{name.ToLowerInvariant()}", $@"Components\VaultBoys\SPECIAL\{name}.swf")).ToList();

        public const string ConditionBodyIdPrefix = "condition/body_";
        public const string ConditionHeadMovie = @"Components\ConditionClips\Condition_Head.swf";

        [GeneratedRegex(@"^Components\\ConditionClips\\Condition_Body_(?<variant>[0-9]+)\.swf$", RegexOptions.IgnoreCase | RegexOptions.CultureInvariant)]
        private static partial Regex ConditionBodyMovie();

        /// <summary>
        /// The limb condition figures the archive holds: the head and every <c>Condition_Body_&lt;n&gt;</c> movie, which must be numbered
        /// from 0 without gaps.
        /// </summary>
        /// <param name="movieSources">The archive paths, below <c>Interface\</c>, of the movies that were read.</param>
        public static IReadOnlyList<MovieClipSource> ConditionMovies(IEnumerable<string> movieSources)
        {
            var sources = movieSources.ToList();
            var bodies = sources.Select(source => (Source: source, Match: ConditionBodyMovie().Match(source)))
                .Where(movie => movie.Match.Success)
                .Select(movie => (movie.Source, Variant: int.Parse(movie.Match.Groups["variant"].Value, CultureInfo.InvariantCulture)))
                .OrderBy(movie => movie.Variant).ToList();
            if (!bodies.Select(body => body.Variant).SequenceEqual(Enumerable.Range(0, bodies.Count)) || bodies.Count == 0)
                throw new PipboyArtException($"The condition body movies ({string.Join(", ", bodies.Select(b => b.Variant))}) are not numbered 0..n-1.");
            string head = sources.SingleOrDefault(source => string.Equals(source, ConditionHeadMovie, StringComparison.OrdinalIgnoreCase))
                ?? throw new PipboyArtException($"Movie '{ConditionHeadMovie}' is missing from the interface archive.");
            return
            [
                .. bodies.Select(body => new MovieClipSource(string.Create(CultureInfo.InvariantCulture, $"{ConditionBodyIdPrefix}{body.Variant:D2}"), body.Source, TightBounds: true)),
                new MovieClipSource("condition/head", head, TightBounds: true),
            ];
        }

        /// <summary>Clip id of a game movie: lowercase, forward slashes, no extension, no trailing blanks in a segment.</summary>
        public static string VaultBoyClipId(string movieRelativePath)
        {
            string withoutExtension = Path.ChangeExtension(movieRelativePath.Replace('\\', '/'), null);
            var segments = withoutExtension.Split('/').Select(segment => segment.Trim().ToLowerInvariant());
            return "vaultboy/" + string.Join('/', segments);
        }

        /// <summary>The Vault Boy movie of the perk card symbol <c>PerkClip&lt;hex&gt;</c>.</summary>
        public static string PerkMoviePath(string cardClassName) => PerkMovieOfHex(cardClassName[PerkCardClassPrefix.Length..]);

        /// <summary>The Vault Boy movie a perk has by naming convention: the perk's form id in lowercase hex.</summary>
        public static string PerkMoviePath(uint formId) => PerkMovieOfHex(formId.ToString("x", CultureInfo.InvariantCulture));

        static string PerkMovieOfHex(string hex) => $"{PerkMovieFolder}{PerkMovieNamePrefix}{hex}.swf";
    }
}
