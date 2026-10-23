using System.IO.Compression;
using System.Text;

namespace AssetConverter.Swf
{
    /// <summary>
    /// Reads a whole SWF movie into a <see cref="SwfMovie"/>: header, character definitions, the main timeline and every sprite timeline,
    /// linkage (symbol classes, exports, imports) and the ActionScript 3 blocks. Malformed data throws <see cref="SwfFormatException"/>;
    /// tags the layout does not need are skipped by their length.
    /// </summary>
    internal static class SwfMovieReader
    {
        private const int HeaderPrefixSize = 8;
        private const int LengthOffset = 4;

        public static SwfMovie Read(byte[] file)
        {
            if (file.Length < HeaderPrefixSize + 1)
                throw new SwfFormatException("The SWF is truncated before its header ends.");
            int version = file[3];
            byte[] body = Body(file);
            uint statedLength = BitConverter.ToUInt32(file, LengthOffset);
            if (statedLength != (uint)(HeaderPrefixSize + body.Length))
                throw new SwfFormatException($"The SWF header states {statedLength} bytes, but the file holds {HeaderPrefixSize + body.Length}.");
            var bits = new SwfBits(body);
            TwipRect stage = bits.Rect();
            double frameRate = bits.U16() / 256.0;
            int frameCount = bits.U16();

            var state = new MovieState();
            var rootTimeline = new TimelineBuilder(frameCount);
            foreach (Tag tag in Tags(body, bits.Position, body.Length - bits.Position, "the main timeline"))
                ReadTopLevelTag(tag, body, rootTimeline, state);

            SwfTimelineTags root = rootTimeline.Build("the main timeline");
            root = WithSceneLabels(root, state.SceneLabels);
            return new SwfMovie(version, stage, frameRate, root, state.Characters, state.SymbolClasses, state.Exports, state.Imports,
                state.ScalingGrids, state.FontNames, state.AbcBlocks, state.TextSettings);
        }

        private static byte[] Body(byte[] file)
        {
            string signature = Encoding.ASCII.GetString(file, 0, 3);
            switch (signature)
            {
                case "FWS":
                case "GFX":
                    return file.AsSpan(HeaderPrefixSize).ToArray();
                case "CWS":
                case "CFX":
                    try
                    {
                        using var inflater = new ZLibStream(new MemoryStream(file, HeaderPrefixSize, file.Length - HeaderPrefixSize), CompressionMode.Decompress);
                        using var output = new MemoryStream();
                        inflater.CopyTo(output);
                        return output.ToArray();
                    }
                    catch (InvalidDataException e)
                    {
                        throw new SwfFormatException($"The compressed SWF body cannot be inflated: {e.Message}");
                    }
                default:
                    throw new SwfFormatException($"Unsupported SWF signature '{signature}'.");
            }
        }

        private readonly record struct Tag(int Code, int Start, int Length);

        private static IEnumerable<Tag> Tags(byte[] data, int start, int length, string where)
        {
            var bits = new SwfBits(data, start, length);
            while (!bits.AtEnd)
            {
                int header = bits.U16();
                int code = header >> 6;
                long tagLength = header & 0x3F;
                if (tagLength == 0x3F)
                    tagLength = bits.U32();
                if (tagLength > bits.Remaining)
                    throw new SwfFormatException($"Tag {code} in {where} claims {tagLength} bytes but {bits.Remaining} are left.");
                if (code == SwfTagCodes.End)
                {
                    if (!bits.AtEnd)
                        throw new SwfFormatException($"{bits.Remaining} bytes follow the End tag of {where}.");
                    yield break;
                }
                yield return new Tag(code, bits.Position, (int)tagLength);
                bits.Skip((int)tagLength);
            }
            throw new SwfFormatException($"{where} has no End tag.");
        }

        private sealed class MovieState
        {
            public Dictionary<int, SwfCharacter> Characters { get; } = [];
            public List<(int, string)> SymbolClasses { get; } = [];
            public List<(int, string)> Exports { get; } = [];
            public Dictionary<int, SwfImport> Imports { get; } = [];
            public Dictionary<int, TwipRect> ScalingGrids { get; } = [];
            public Dictionary<int, string> FontNames { get; } = [];
            public List<SwfAbcBlock> AbcBlocks { get; } = [];
            public Dictionary<int, SwfTextSettings> TextSettings { get; } = [];
            public List<(int Frame, string Label)> SceneLabels { get; } = [];

            public void Add(SwfCharacter character)
            {
                if (!Characters.TryAdd(character.Id, character))
                    throw new SwfFormatException($"Character {character.Id} is defined twice.");
            }
        }

        private static void ReadTopLevelTag(Tag tag, byte[] body, TimelineBuilder root, MovieState state)
        {
            var bits = new SwfBits(body, tag.Start, tag.Length);
            if (root.TryRead(tag.Code, bits))
                return;
            switch (tag.Code)
            {
                case SwfTagCodes.DefineShape:
                case SwfTagCodes.DefineShape2:
                case SwfTagCodes.DefineShape3:
                case SwfTagCodes.DefineShape4:
                    state.Add(SwfShapeReader.Read(bits, ShapeVersion(tag.Code)));
                    break;
                case SwfTagCodes.DefineSprite:
                    state.Add(ReadSprite(bits, body));
                    break;
                case SwfTagCodes.DefineEditText:
                    state.Add(SwfTextReader.ReadEditText(bits));
                    break;
                case SwfTagCodes.DefineText:
                case SwfTagCodes.DefineText2:
                    state.Add(SwfTextReader.ReadStaticText(bits, withAlpha: tag.Code == SwfTagCodes.DefineText2));
                    break;
                case SwfTagCodes.DefineFont2:
                case SwfTagCodes.DefineFont3:
                    state.Add(SwfTextReader.ReadFont(bits, tag.Code == SwfTagCodes.DefineFont3 ? 3 : 2));
                    break;
                case SwfTagCodes.DefineFont4:
                    state.Add(SwfTextReader.ReadFont4(bits));
                    break;
                case SwfTagCodes.DefineFontName:
                    state.FontNames[bits.U16()] = bits.String();
                    break;
                case SwfTagCodes.SymbolClass:
                    state.SymbolClasses.AddRange(ReadIdNamePairs(bits));
                    break;
                case SwfTagCodes.ExportAssets:
                    state.Exports.AddRange(ReadIdNamePairs(bits));
                    break;
                case SwfTagCodes.ImportAssets:
                case SwfTagCodes.ImportAssets2:
                    ReadImports(bits, tag.Code == SwfTagCodes.ImportAssets2, state.Imports);
                    break;
                case SwfTagCodes.DefineScalingGrid:
                    state.ScalingGrids[bits.U16()] = bits.Rect();
                    break;
                case SwfTagCodes.CsmTextSettings:
                    {
                        int textId = bits.U16();
                        int renderer = (int)bits.UB(2);
                        int gridFit = (int)bits.UB(3);
                        bits.UB(3);
                        state.TextSettings[textId] = new SwfTextSettings(renderer, gridFit, bits.Float32(), bits.Float32());
                        break;
                    }
                case SwfTagCodes.DoAbc:
                    bits.U32();
                    state.AbcBlocks.Add(new SwfAbcBlock(bits.String(), bits.RestOfSpan()));
                    break;
                case SwfTagCodes.DoAbcWithoutHeader:
                    state.AbcBlocks.Add(new SwfAbcBlock("", bits.RestOfSpan()));
                    break;
                case SwfTagCodes.DefineSceneAndFrameLabelData:
                    ReadSceneLabels(bits, state.SceneLabels);
                    break;
                case SwfTagCodes.DefineBitsLossless:
                case SwfTagCodes.DefineBitsLossless2:
                    int bitmapId = bits.U16();
                    bits.U8();
                    state.Add(new SwfOtherCharacter(bitmapId, "bitmap", bits.U16(), bits.U16()));
                    break;
                case SwfTagCodes.DefineExternalImage:
                    state.Add(new SwfOtherCharacter(bits.U16(), "bitmap"));
                    break;
                default:
                    if (SwfTagCodes.OtherCharacterKinds.TryGetValue(tag.Code, out string? kind))
                        state.Add(new SwfOtherCharacter(bits.U16(), kind));
                    break;
            }
        }

        private static int ShapeVersion(int code) => code switch
        {
            SwfTagCodes.DefineShape => 1,
            SwfTagCodes.DefineShape2 => 2,
            SwfTagCodes.DefineShape3 => 3,
            _ => 4,
        };

        private static SwfSprite ReadSprite(SwfBits bits, byte[] body)
        {
            int id = bits.U16();
            int frameCount = bits.U16();
            var timeline = new TimelineBuilder(frameCount);
            foreach (Tag tag in Tags(body, bits.Position, bits.Remaining, $"sprite {id}"))
                timeline.TryRead(tag.Code, new SwfBits(body, tag.Start, tag.Length));
            return new SwfSprite(id, timeline.Build($"sprite {id}"));
        }

        private static List<(int, string)> ReadIdNamePairs(SwfBits bits)
        {
            int count = bits.U16();
            var pairs = new List<(int, string)>(count);
            for (int i = 0; i < count; i++)
                pairs.Add((bits.U16(), bits.String()));
            return pairs;
        }

        private static void ReadImports(SwfBits bits, bool version2, Dictionary<int, SwfImport> imports)
        {
            string url = bits.String();
            if (version2)
            {
                bits.U8();
                bits.U8();
            }
            foreach ((int id, string name) in ReadIdNamePairs(bits))
                imports[id] = new SwfImport(url, name);
        }

        private static void ReadSceneLabels(SwfBits bits, List<(int Frame, string Label)> labels)
        {
            uint scenes = bits.EncodedU32();
            for (uint i = 0; i < scenes; i++)
            {
                bits.EncodedU32();
                bits.String();
            }
            uint count = bits.EncodedU32();
            for (uint i = 0; i < count; i++)
                labels.Add((checked((int)bits.EncodedU32()), bits.String()));
        }

        /// <summary>Adds the scene-data labels (0-based frame offsets) a movie may carry in place of, or besides, FrameLabel tags.</summary>
        private static SwfTimelineTags WithSceneLabels(SwfTimelineTags root, List<(int Frame, string Label)> sceneLabels)
        {
            if (sceneLabels.Count == 0)
                return root;
            var frames = root.Frames.ToList();
            foreach ((int frame, string label) in sceneLabels)
            {
                if (frame >= frames.Count)
                    throw new SwfFormatException($"Frame label '{label}' names frame {frame + 1}, but the main timeline has {frames.Count} frames.");
                if (!frames[frame].Labels.Contains(label))
                    frames[frame] = frames[frame] with { Labels = [.. frames[frame].Labels, label] };
            }
            return root with { Frames = frames };
        }

        /// <summary>Collects a timeline's control tags into frames, each ending with its ShowFrame.</summary>
        private sealed class TimelineBuilder(int declaredFrameCount)
        {
            private readonly List<SwfFrame> _frames = [];
            private List<string> _labels = [];
            private List<object> _actions = [];

            /// <returns>Whether the tag is a timeline control tag (and was consumed).</returns>
            public bool TryRead(int code, SwfBits bits)
            {
                switch (code)
                {
                    case SwfTagCodes.ShowFrame:
                        _frames.Add(new SwfFrame(_labels, _actions));
                        _labels = [];
                        _actions = [];
                        return true;
                    case SwfTagCodes.FrameLabel:
                        _labels.Add(bits.String());
                        return true;
                    case SwfTagCodes.PlaceObject:
                        _actions.Add(SwfPlaceObjectReader.ReadPlaceObject1(bits));
                        return true;
                    case SwfTagCodes.PlaceObject2:
                        _actions.Add(SwfPlaceObjectReader.ReadPlaceObject2(bits));
                        return true;
                    case SwfTagCodes.PlaceObject3:
                        _actions.Add(SwfPlaceObjectReader.ReadPlaceObject3(bits));
                        return true;
                    case SwfTagCodes.PlaceObject4:
                        throw new SwfFormatException("PlaceObject4 is not supported.");
                    case SwfTagCodes.RemoveObject:
                        bits.U16();
                        _actions.Add(new SwfRemoveObject(bits.U16()));
                        return true;
                    case SwfTagCodes.RemoveObject2:
                        _actions.Add(new SwfRemoveObject(bits.U16()));
                        return true;
                    default:
                        return false;
                }
            }

            public SwfTimelineTags Build(string where)
            {
                if (_labels.Count > 0 || _actions.Count > 0)
                    throw new SwfFormatException($"{where} has display list changes or labels after its last frame.");
                if (_frames.Count != declaredFrameCount)
                    throw new SwfFormatException($"{where} declares {declaredFrameCount} frames but holds {_frames.Count}.");
                return new SwfTimelineTags(declaredFrameCount, _frames);
            }
        }
    }

    internal static class SwfTagCodes
    {
        public const int End = 0;
        public const int ShowFrame = 1;
        public const int DefineShape = 2;
        public const int PlaceObject = 4;
        public const int RemoveObject = 5;
        public const int DefineText = 11;
        public const int DefineBitsLossless = 20;
        public const int DefineShape2 = 22;
        public const int PlaceObject2 = 26;
        public const int RemoveObject2 = 28;
        public const int DefineShape3 = 32;
        public const int DefineText2 = 33;
        public const int DefineBitsLossless2 = 36;
        public const int DefineEditText = 37;
        public const int DefineSprite = 39;
        public const int FrameLabel = 43;
        public const int DefineFont2 = 48;
        public const int ExportAssets = 56;
        public const int ImportAssets = 57;
        public const int PlaceObject3 = 70;
        public const int ImportAssets2 = 71;
        public const int DoAbcWithoutHeader = 72;
        public const int CsmTextSettings = 74;
        public const int DefineFont3 = 75;
        public const int SymbolClass = 76;
        public const int DefineScalingGrid = 78;
        public const int DoAbc = 82;
        public const int DefineShape4 = 83;
        public const int DefineSceneAndFrameLabelData = 86;
        public const int DefineFontName = 88;
        public const int DefineFont4 = 91;
        public const int PlaceObject4 = 94;
        public const int DefineExternalImage = 1001;

        /// <summary>Definition tags whose first field is the character id and whose content the layout does not need.</summary>
        public static readonly IReadOnlyDictionary<int, string> OtherCharacterKinds = new Dictionary<int, string>
        {
            [6] = "bitmap",
            [7] = "button",
            [10] = "font",
            [14] = "sound",
            [21] = "bitmap",
            [34] = "button",
            [35] = "bitmap",
            [46] = "morphshape",
            [60] = "video",
            [84] = "morphshape",
            [87] = "binary",
            [90] = "bitmap",
        };
    }
}
