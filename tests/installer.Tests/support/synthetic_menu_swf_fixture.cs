using AssetConverter;
using AssetConverter.Swf;

namespace InstallerTests.Support
{
    /// <summary>
    /// A miniature menu SWF with its ActionScript, made of placeholder names and numbers only. Each property is one thing a test can
    /// change; <see cref="Build"/> writes the SWF, whose layout the UI layout extractor converts.
    /// </summary>
    internal sealed record SyntheticMenuSwfFixture
    {
        public const string FontConfig =
            "fontlib \"Interface\\synth_fonts.swf\"\r\n" +
            "map \"$SYNTH_Font\" = \"Synth Family\" Normal\r\n" +
            "map \"$SYNTH_Font_Bold\" = \"Synth Family Bold\" Bold\r\n";

        public const int MenuSprite = 100;
        public const int PanelSprite = 110;
        public const int BackgroundSprite = 111;
        public const int BackgroundShape = 112;
        public const int EntrySprite = 20;
        public const int BorderSprite = 21;
        public const int BorderShape = 22;
        public const int EntryText = 25;
        public const int ListSprite = 66;
        public const int HolderSprite = 90;

        public SwfMatrix MenuMatrix { get; init; } = SwfPlacement.Translate(1000, 2000);
        public SwfMatrix BorderMatrix { get; init; } = new(2, 0, 0, 0.5, 0, 0);
        public SwfMatrix BorderShapeMatrix { get; init; } = new(5, 0, 0, 5, 500, 500);
        public SwfMatrix TextMatrix { get; init; } = SwfPlacement.Translate(100, 30);
        public int? FontHeight { get; init; } = 300;
        public string ListName { get; init; } = "Items_mc";
        public string HolderName { get; init; } = "Holder_mc";
        public string BorderName { get; init; } = "frame";
        public bool EntryHasSymbolClass { get; init; } = true;

        /// <summary>Colour transforms by instance name (or <c>"112"</c> for the background shape's placement).</summary>
        public IReadOnlyDictionary<string, ColorTransform> Colors { get; init; } = new Dictionary<string, ColorTransform>();

        /// <summary>Placements a test adds to a sprite's first frame, by sprite id.</summary>
        public IReadOnlyDictionary<int, Action<SwfTimelineWriter>> FirstFrameExtras { get; init; } = new Dictionary<int, Action<SwfTimelineWriter>>();

        /// <summary>A second frame a test appends to a sprite (by id, or 0 for the main timeline).</summary>
        public IReadOnlyDictionary<int, Action<SwfTimelineWriter>> LaterFrames { get; init; } = new Dictionary<int, Action<SwfTimelineWriter>>();

        public bool UsePlaceObject2 { get; init; }

        public string PanelClass { get; init; } = "Synth.Panel";
        public string EntryClass { get; init; } = "SynthEntry";

        /// <summary>The class the entry extends, as package and name; <c>null</c> for none.</summary>
        public (string Package, string Name)? EntryBase { get; init; } = ("Lib", "FadeEntry");

        /// <summary>The constants the panel script assigns to the list, by property; a value of <c>null</c> leaves the assignment out.</summary>
        public IReadOnlyDictionary<string, object?> ListConstants { get; init; } = new Dictionary<string, object?>
        {
            ["lockSelection"] = false,
            ["entryClass"] = "SynthEntry",
            ["itemCount"] = 7,
            ["gap"] = 2.5,
        };

        /// <summary>A property the panel script assigns a second time (in a second method).</summary>
        public (string Property, object Value)? SecondListAssignment { get; init; }

        /// <summary>The entry's text colours: disabled selected and unselected, enabled selected and unselected.</summary>
        public IReadOnlyList<object> TextColors { get; init; } = [0x112233, 5592405, 255, 65280];

        public double DisabledSelectedBorderAlpha { get; init; } = 0.25;
        public bool ColorAssignedInBothStates { get; init; } = true;
        public double? UnfocusedAlpha { get; init; } = 0.4;
        public double? SelectedRectAlpha { get; init; } = 0.75;

        public byte[] Build()
        {
            var swf = new SwfMovieBuilder { Stage = new TwipRect(0, 0, 19200, 10800), FrameRate = 30 };
            swf.EditText(EntryText, new TwipRect(0, 0, 2000, 400), fontClass: "$SYNTH_Font", fontHeight: FontHeight);
            swf.Shape(BorderShape, new TwipRect(-100, -100, 100, 100), new SwfColor(255, 128, 0, 255), version: 1);
            Sprite(swf, BorderSprite, new SwfPlacement(1) { Character = BorderShape, Matrix = BorderShapeMatrix });
            Sprite(swf, EntrySprite,
                new SwfPlacement(1) { Character = BorderSprite, Name = BorderName, Matrix = BorderMatrix },
                new SwfPlacement(3) { Character = EntryText, Name = "textField", Matrix = TextMatrix });
            swf.Shape(BackgroundShape, new TwipRect(-10, -10, 2010, 1010), new SwfColor(10, 20, 30, 51), version: 4, edgeBounds: new TwipRect(0, 0, 2000, 1000));
            Sprite(swf, BackgroundSprite, new SwfPlacement(1) { Character = BackgroundShape, Matrix = SwfMatrix.Identity, Color = Colors.GetValueOrDefault("112") });
            swf.Sprite(ListSprite, timeline => timeline.ShowFrame());
            swf.Sprite(HolderSprite, timeline => timeline.ShowFrame());
            Sprite(swf, PanelSprite,
                Named(new SwfPlacement(1) { Character = BackgroundSprite, Matrix = new SwfMatrix(1, 0, 0, 2, -50, -60) }, "Backdrop_mc"),
                Named(new SwfPlacement(8) { Character = ListSprite, Matrix = SwfPlacement.Translate(-100, -200) }, ListName));
            Sprite(swf, MenuSprite,
                Named(new SwfPlacement(1) { Character = PanelSprite, Matrix = SwfPlacement.Translate(400, 600) }, "Panel_mc"),
                Named(new SwfPlacement(2) { Character = HolderSprite, Matrix = SwfPlacement.Translate(3000, 5000) }, HolderName));
            swf.SymbolClass(EntryHasSymbolClass ? [(PanelSprite, PanelClass), (EntrySprite, EntryClass)] : [(PanelSprite, PanelClass)]);
            swf.Abc("synth", Scripts());

            Place(swf.Root, Named(new SwfPlacement(1) { Character = MenuSprite, Matrix = MenuMatrix }, "Root_mc"));
            swf.Root.ShowFrame();
            if (LaterFrames.TryGetValue(0, out var laterRoot))
            {
                laterRoot(swf.Root);
                swf.Root.ShowFrame();
            }
            return swf.Build();
        }

        public SwfMovie Movie() => SwfMovieReader.Read(Build());

        private SwfPlacement Named(SwfPlacement placement, string name) => placement with { Name = name, Color = Colors.GetValueOrDefault(name) };

        private void Place(SwfTimelineWriter timeline, SwfPlacement placement)
        {
            if (UsePlaceObject2)
                timeline.Place2(placement);
            else
                timeline.Place(placement);
        }

        private void Sprite(SwfMovieBuilder swf, int id, params SwfPlacement[] placements) => swf.Sprite(id, timeline =>
        {
            foreach (SwfPlacement placement in placements)
                Place(timeline, placement);
            if (FirstFrameExtras.TryGetValue(id, out var extras))
                extras(timeline);
            timeline.ShowFrame();
            if (LaterFrames.TryGetValue(id, out var later))
            {
                later(timeline);
                timeline.ShowFrame();
            }
        });

        private byte[] Scripts()
        {
            var abc = new AbcBuilder();
            int listName = abc.Name("Items_mc");

            var panelCode = new AbcCode();
            foreach ((string property, object? value) in ListConstants)
                if (value is not null)
                    Assign(abc, panelCode.GetLocal0().GetProperty(listName), value).SetProperty(abc.Name(property));
            int setProps = abc.Function(panelCode.ReturnVoid().ToArray());
            var panelMethods = new List<AbcTraitSpec> { new AbcMethodTraitSpec(abc.Name("__setProp_List_mc_Panel_0"), setProps) };
            if (SecondListAssignment is { } second)
            {
                var again = Assign(abc, new AbcCode().GetLocal0().GetProperty(listName), second.Value).SetProperty(abc.Name(second.Property)).ReturnVoid();
                panelMethods.Add(new AbcMethodTraitSpec(abc.Name("frame2"), abc.Function(again.ToArray())));
            }
            (string panelPackage, string panelName) = Split(PanelClass);
            abc.Class(panelPackage, panelName, 0, abc.Method(), abc.Method(), panelMethods);

            (string entryPackage, string entryName) = Split(EntryClass);
            int entryBase = EntryBase is { } b ? abc.Name(b.Name, b.Package) : 0;
            abc.Class(entryPackage, entryName, entryBase, abc.Method(), abc.Method(),
                [new AbcMethodTraitSpec(abc.Name("SetEntryText"), abc.Function(EntryStateCode(abc), parameters: 2, localCount: 4))]);

            if (EntryBase is { } fade)
            {
                var traits = UnfocusedAlpha is { } alpha
                    ? new List<AbcTraitSpec> { new AbcSlotSpec(abc.Name("fRestingFrameAlpha", fade.Package, AbcBuilder.PrivateNamespace), IsConst: true, abc.Double(alpha), AbcBuilder.ConstantDouble) }
                    : [];
                abc.Class(fade.Package, fade.Name, 0, abc.Method(), abc.Method(), traits);
            }
            if (SelectedRectAlpha is { } rectAlpha)
                abc.Class("Shared", "GlobalFunc", 0, abc.Method(), abc.Method(), classTraits:
                    [new AbcSlotSpec(abc.Name("SELECTED_RECT_ALPHA"), IsConst: true, abc.Double(rectAlpha), AbcBuilder.ConstantDouble)]);
            return abc.Build();
        }

        /// <summary>
        /// <c>if (entry.disabled) { border.alpha = selected ? a : 0; textField.textColor = selected ? c0 : c1; }
        /// else { border.alpha = selected ? resting : 0; textField.textColor = selected ? c2 : c3; }</c>
        /// </summary>
        private byte[] EntryStateCode(AbcBuilder abc)
        {
            int border = abc.Name("frame"), alpha = abc.Name("alpha"), selected = abc.Name("selected");
            int textField = abc.Name("textField"), textColor = abc.Name("textColor");
            var code = new AbcCode();
            code.GetLocal(1).GetProperty(abc.Name("disabled"));
            int toEnabled = code.IfFalse();
            Ternary(abc, code.GetLocal0().GetProperty(border), selected, DisabledSelectedBorderAlpha, 0).SetProperty(alpha);
            Ternary(abc, code.GetLocal0().GetProperty(textField), selected, TextColors[0], TextColors[1]).SetProperty(textColor);
            int toEnd = code.Jump();
            code.Here(toEnabled);
            code.GetLocal0().GetProperty(border).GetLocal0().GetProperty(selected);
            int restingElse = code.IfFalse();
            code.GetLocal(3);
            int restingEnd = code.Jump();
            code.Here(restingElse).PushByte(0).Here(restingEnd).SetProperty(alpha);
            if (ColorAssignedInBothStates)
                Ternary(abc, code.GetLocal0().GetProperty(textField), selected, TextColors[2], TextColors[3]).SetProperty(textColor);
            code.Here(toEnd);
            return code.ReturnVoid().ToArray();
        }

        private static AbcCode Ternary(AbcBuilder abc, AbcCode code, int condition, object whenTrue, object whenFalse)
        {
            code.GetLocal0().GetProperty(condition);
            int otherwise = code.IfFalse();
            Push(abc, code, whenTrue);
            int end = code.Jump();
            code.Here(otherwise);
            Push(abc, code, whenFalse);
            return code.Here(end);
        }

        private static AbcCode Assign(AbcBuilder abc, AbcCode code, object value) => Push(abc, code, value);

        private static AbcCode Push(AbcBuilder abc, AbcCode code, object value) => value switch
        {
            bool flag => flag ? code.PushTrue() : code.Op(0x27),
            int number => code.PushInt(abc.Int(number)),
            double number => code.PushDouble(abc.Double(number)),
            string text => code.PushString(abc.String(text)),
            _ => throw new ArgumentException($"No push for {value.GetType().Name}."),
        };

        private static (string Package, string Name) Split(string qualified)
        {
            int dot = qualified.LastIndexOf('.');
            return dot < 0 ? ("", qualified) : (qualified[..dot], qualified[(dot + 1)..]);
        }
    }
}
