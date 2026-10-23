namespace AssetConverter.Swf
{
    /// <summary>Reads PlaceObject 1 to 3, including the PlaceObject3 filter list, blend mode and visibility.</summary>
    internal static class SwfPlaceObjectReader
    {
        public static SwfPlaceObject ReadPlaceObject1(SwfBits bits)
        {
            int characterId = bits.U16();
            int depth = bits.U16();
            SwfMatrix matrix = bits.Matrix();
            ColorTransform? color = bits.AtEnd ? null : bits.ColorTransform(withAlpha: false);
            return new SwfPlaceObject(depth, Move: false, CharacterId: characterId, Matrix: matrix, Color: color);
        }

        public static SwfPlaceObject ReadPlaceObject2(SwfBits bits)
        {
            int flags = bits.U8();
            return ReadCommon(bits, flags, flags2: 0);
        }

        public static SwfPlaceObject ReadPlaceObject3(SwfBits bits)
        {
            int flags = bits.U8();
            int flags2 = bits.U8();
            return ReadCommon(bits, flags, flags2, isVersion3: true);
        }

        private static SwfPlaceObject ReadCommon(SwfBits bits, int flags, int flags2, bool isVersion3 = false)
        {
            bool hasClipActions = (flags & 0x80) != 0;
            bool hasClipDepth = (flags & 0x40) != 0;
            bool hasName = (flags & 0x20) != 0;
            bool hasRatio = (flags & 0x10) != 0;
            bool hasColor = (flags & 0x08) != 0;
            bool hasMatrix = (flags & 0x04) != 0;
            bool hasCharacter = (flags & 0x02) != 0;
            bool move = (flags & 0x01) != 0;
            bool hasBackground = (flags2 & 0x40) != 0;
            bool hasVisible = (flags2 & 0x20) != 0;
            bool hasImage = (flags2 & 0x10) != 0;
            bool hasClassName = (flags2 & 0x08) != 0;
            bool hasCacheAsBitmap = (flags2 & 0x04) != 0;
            bool hasBlendMode = (flags2 & 0x02) != 0;
            bool hasFilters = (flags2 & 0x01) != 0;

            int depth = bits.U16();
            // The file format specification states the class name for "image and character", but files (and the Ruffle player's reader) carry it
            // for "image without character": a placement of an image character has no class name field.
            string? className = isVersion3 && (hasClassName || (hasImage && !hasCharacter)) ? bits.String() : null;
            int? characterId = hasCharacter ? bits.U16() : null;
            SwfMatrix? matrix = hasMatrix ? bits.Matrix() : null;
            ColorTransform? color = hasColor ? bits.ColorTransform(withAlpha: true) : null;
            int? ratio = hasRatio ? bits.U16() : null;
            string? name = hasName ? bits.String() : null;
            int? clipDepth = hasClipDepth ? bits.U16() : null;
            IReadOnlyList<SwfFilter>? filters = hasFilters ? Filters(bits) : null;
            SwfBlendMode? blend = hasBlendMode ? BlendMode(bits.U8()) : null;
            bool? cacheAsBitmap = hasCacheAsBitmap ? bits.U8() != 0 : null;
            bool? visible = hasVisible ? bits.U8() != 0 : null;
            SwfColor? background = hasBackground ? bits.Rgba() : null;
            if (hasClipActions)
                throw new SwfFormatException($"The placement at depth {depth} carries clip actions, which are not supported.");
            bits.Align();
            if (!bits.AtEnd)
                throw new SwfFormatException($"The placement at depth {depth} has {bits.Remaining} bytes left after its last field; its flags do not describe it.");
            return new SwfPlaceObject(depth, move, characterId, name, matrix, color, ratio, clipDepth, filters, blend, cacheAsBitmap, visible, className, background);
        }

        private static SwfBlendMode BlendMode(int value) => value switch
        {
            0 => SwfBlendMode.Normal,
            >= (int)SwfBlendMode.Normal and <= (int)SwfBlendMode.Hardlight => (SwfBlendMode)value,
            _ => throw new SwfFormatException($"Blend mode {value} is not a SWF blend mode."),
        };

        private static List<SwfFilter> Filters(SwfBits bits)
        {
            int count = bits.U8();
            var filters = new List<SwfFilter>(count);
            for (int i = 0; i < count; i++)
                filters.Add(Filter(bits));
            return filters;
        }

        private static SwfFilter Filter(SwfBits bits)
        {
            int type = bits.U8();
            switch (type)
            {
                case 0:
                    {
                        SwfColor color = bits.Rgba();
                        double blurX = bits.Fixed(), blurY = bits.Fixed(), angle = bits.Fixed(), distance = bits.Fixed(), strength = bits.Fixed8();
                        bool inner = bits.Flag(), knockout = bits.Flag(), composite = bits.Flag();
                        int passes = (int)bits.UB(5);
                        return new DropShadowFilter(color, blurX, blurY, angle, distance, strength, inner, knockout, composite, passes);
                    }
                case 1:
                    {
                        double blurX = bits.Fixed(), blurY = bits.Fixed();
                        int passes = (int)bits.UB(5);
                        bits.UB(3);
                        return new BlurFilter(blurX, blurY, passes);
                    }
                case 2:
                    {
                        SwfColor color = bits.Rgba();
                        double blurX = bits.Fixed(), blurY = bits.Fixed(), strength = bits.Fixed8();
                        bool inner = bits.Flag(), knockout = bits.Flag(), composite = bits.Flag();
                        int passes = (int)bits.UB(5);
                        return new GlowFilter(color, blurX, blurY, strength, inner, knockout, composite, passes);
                    }
                case 3:
                    {
                        // The file format specification lists the shadow first; files (and the Ruffle player's reader) state the highlight first.
                        SwfColor highlight = bits.Rgba(), shadow = bits.Rgba();
                        double blurX = bits.Fixed(), blurY = bits.Fixed(), angle = bits.Fixed(), distance = bits.Fixed(), strength = bits.Fixed8();
                        bool inner = bits.Flag(), knockout = bits.Flag(), composite = bits.Flag(), onTop = bits.Flag();
                        int passes = (int)bits.UB(4);
                        return new BevelFilter(shadow, highlight, blurX, blurY, angle, distance, strength, inner, knockout, composite, onTop, passes);
                    }
                case 4:
                case 7:
                    {
                        int count = bits.U8();
                        var colors = new List<SwfColor>(count);
                        for (int i = 0; i < count; i++)
                            colors.Add(bits.Rgba());
                        var ratios = new List<int>(count);
                        for (int i = 0; i < count; i++)
                            ratios.Add(bits.U8());
                        double blurX = bits.Fixed(), blurY = bits.Fixed(), angle = bits.Fixed(), distance = bits.Fixed(), strength = bits.Fixed8();
                        bool inner = bits.Flag(), knockout = bits.Flag(), composite = bits.Flag(), onTop = bits.Flag();
                        int passes = (int)bits.UB(4);
                        return new GradientFilter(type == 7, new EquatableList<SwfColor>(colors), new EquatableList<int>(ratios), blurX, blurY, angle, distance, strength, inner, knockout, composite, onTop, passes);
                    }
                case 5:
                    {
                        int matrixX = bits.U8(), matrixY = bits.U8();
                        float divisor = bits.Float32(), bias = bits.Float32();
                        var matrix = new List<float>(matrixX * matrixY);
                        for (int i = 0; i < matrixX * matrixY; i++)
                            matrix.Add(bits.Float32());
                        SwfColor defaultColor = bits.Rgba();
                        bits.UB(6);
                        bool clamp = bits.Flag(), preserveAlpha = bits.Flag();
                        return new ConvolutionFilter(matrixX, matrixY, divisor, bias, new EquatableList<float>(matrix), defaultColor, clamp, preserveAlpha);
                    }
                case 6:
                    {
                        const int ColorMatrixSize = 20;
                        var matrix = new List<float>(ColorMatrixSize);
                        for (int i = 0; i < ColorMatrixSize; i++)
                            matrix.Add(bits.Float32());
                        return new ColorMatrixFilter(new EquatableList<float>(matrix));
                    }
                default:
                    throw new SwfFormatException($"Filter type {type} is not a SWF filter.");
            }
        }
    }
}
