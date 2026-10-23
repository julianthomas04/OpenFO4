namespace AssetConverter.Swf
{
    /// <summary>
    /// Reads DefineShape 1 to 4: bounds, and every fill and line style, including the style arrays a shape introduces part-way through its
    /// records, and the outline as runs of edges with the styles they were drawn in.
    /// </summary>
    internal static class SwfShapeReader
    {
        public static SwfShape Read(SwfBits bits, int version)
        {
            int id = bits.U16();
            TwipRect bounds = bits.Rect();
            TwipRect? edgeBounds = null;
            SwfShapeFlags? flags = null;
            if (version == 4)
            {
                edgeBounds = bits.Rect();
                flags = ReadFlags(bits.U8(), id);
            }

            var fills = new List<SwfFillStyle>();
            var lines = new List<SwfLineStyle>();
            fills.AddRange(FillStyles(bits, version));
            lines.AddRange(LineStyles(bits, version));
            List<SwfShapePath> paths = ReadRecords(bits, version, fills, lines);
            return new SwfShape(id, version, bounds, edgeBounds, fills, lines, paths, flags);
        }

        private static SwfShapeFlags ReadFlags(byte flags, int shapeId)
        {
            const int Reserved = 0xF8;
            const int FillWindingRule = 0x04;
            const int NonScalingStrokes = 0x02;
            const int ScalingStrokes = 0x01;
            if ((flags & Reserved) != 0)
                throw new SwfFormatException($"Shape {shapeId} sets the reserved bits of its flags (0x{flags:x2}).");
            return new SwfShapeFlags((flags & FillWindingRule) != 0, (flags & NonScalingStrokes) != 0, (flags & ScalingStrokes) != 0);
        }

        private static List<SwfShapePath> ReadRecords(SwfBits bits, int version, List<SwfFillStyle> fills, List<SwfLineStyle> lines)
        {
            var paths = new List<SwfShapePath>();
            var edges = new List<SwfEdge>();
            SwfPoint position = default;
            SwfPoint pathStart = default;
            int fillLeft = 0, fillRight = 0, line = 0;
            int fillBase = 0, lineBase = 0;
            bits.Align();
            int fillBits = (int)bits.UB(4);
            int lineBits = (int)bits.UB(4);

            void EndPath()
            {
                if (edges.Count > 0)
                    paths.Add(new SwfShapePath(fillLeft, fillRight, line, pathStart, edges.ToArray()));
                edges.Clear();
                pathStart = position;
            }

            while (true)
            {
                if (bits.Flag())
                {
                    SwfEdge edge = ReadEdge(bits, position);
                    edges.Add(edge);
                    position = edge.End;
                    continue;
                }
                bool newStyles = bits.Flag();
                bool lineStyle = bits.Flag();
                bool fillStyle1 = bits.Flag();
                bool fillStyle0 = bits.Flag();
                bool moveTo = bits.Flag();
                if (!newStyles && !lineStyle && !fillStyle1 && !fillStyle0 && !moveTo)
                {
                    EndPath();
                    return paths;
                }
                EndPath();
                if (moveTo)
                {
                    int moveBits = (int)bits.UB(5);
                    position = new SwfPoint(bits.SB(moveBits), bits.SB(moveBits));
                    pathStart = position;
                }
                int newFillLeft = fillStyle0 ? (int)bits.UB(fillBits) : -1;
                int newFillRight = fillStyle1 ? (int)bits.UB(fillBits) : -1;
                int newLine = lineStyle ? (int)bits.UB(lineBits) : -1;
                if (newStyles)
                {
                    fillBase = fills.Count;
                    lineBase = lines.Count;
                    fills.AddRange(FillStyles(bits, version));
                    lines.AddRange(LineStyles(bits, version));
                    bits.Align();
                    fillBits = (int)bits.UB(4);
                    lineBits = (int)bits.UB(4);
                    fillLeft = fillRight = line = 0;
                }
                fillLeft = StyleNumber(newFillLeft, fillBase, fillLeft, fills.Count, "fill");
                fillRight = StyleNumber(newFillRight, fillBase, fillRight, fills.Count, "fill");
                line = StyleNumber(newLine, lineBase, line, lines.Count, "line");
            }
        }

        /// <summary>The style a record selects, numbered across every style array of the shape; <paramref name="selected"/> is -1 when the record leaves the style as it is.</summary>
        private static int StyleNumber(int selected, int arrayStart, int current, int styleCount, string kind)
        {
            if (selected < 0)
                return current;
            if (selected == 0)
                return 0;
            int number = arrayStart + selected;
            return number <= styleCount ? number : throw new SwfFormatException($"A shape record selects {kind} style {selected}, but its array holds {styleCount - arrayStart}.");
        }

        private static SwfEdge ReadEdge(SwfBits bits, SwfPoint from)
        {
            bool straight = bits.Flag();
            int numBits = (int)bits.UB(4) + 2;
            if (!straight)
            {
                var control = Offset(from, bits.SB(numBits), bits.SB(numBits));
                return new SwfEdge(control, Offset(control, bits.SB(numBits), bits.SB(numBits)));
            }
            if (bits.Flag())
                return new SwfEdge(null, Offset(from, bits.SB(numBits), bits.SB(numBits)));
            bool vertical = bits.Flag();
            int delta = bits.SB(numBits);
            return new SwfEdge(null, vertical ? Offset(from, 0, delta) : Offset(from, delta, 0));
        }

        private static SwfPoint Offset(SwfPoint from, int dx, int dy)
        {
            try
            {
                return new SwfPoint(checked(from.X + dx), checked(from.Y + dy));
            }
            catch (OverflowException)
            {
                throw new SwfFormatException($"An edge from ({from.X}, {from.Y}) by ({dx}, {dy}) leaves the range of coordinates.");
            }
        }

        private static int StyleCount(SwfBits bits, int version)
        {
            int count = bits.U8();
            return count == 0xFF && version >= 2 ? bits.U16() : count;
        }

        private static List<SwfFillStyle> FillStyles(SwfBits bits, int version)
        {
            int count = StyleCount(bits, version);
            var styles = new List<SwfFillStyle>(count);
            for (int i = 0; i < count; i++)
                styles.Add(FillStyle(bits, version));
            return styles;
        }

        public static SwfFillStyle FillStyle(SwfBits bits, int version)
        {
            int type = bits.U8();
            switch (type)
            {
                case (int)SwfFillType.Solid:
                    return new SwfFillStyle(SwfFillType.Solid, Color: Color(bits, version));
                case (int)SwfFillType.LinearGradient:
                case (int)SwfFillType.RadialGradient:
                case (int)SwfFillType.FocalRadialGradient:
                    SwfMatrix matrix = bits.Matrix();
                    return new SwfFillStyle((SwfFillType)type, Matrix: matrix, Gradient: Gradient(bits, version, type == (int)SwfFillType.FocalRadialGradient));
                case (int)SwfFillType.RepeatingBitmap:
                case (int)SwfFillType.ClippedBitmap:
                case (int)SwfFillType.NonSmoothedRepeatingBitmap:
                case (int)SwfFillType.NonSmoothedClippedBitmap:
                    int bitmapId = bits.U16();
                    return new SwfFillStyle((SwfFillType)type, Matrix: bits.Matrix(), BitmapId: bitmapId);
                default:
                    throw new SwfFormatException($"Fill style type 0x{type:x2} is not a SWF fill type.");
            }
        }

        private static SwfGradient Gradient(SwfBits bits, int version, bool focal)
        {
            int spread = (int)bits.UB(2);
            int interpolation = (int)bits.UB(2);
            int count = (int)bits.UB(4);
            var stops = new List<(int, SwfColor)>(count);
            for (int i = 0; i < count; i++)
            {
                int ratio = bits.U8();
                stops.Add((ratio, Color(bits, version)));
            }
            return new SwfGradient(spread, interpolation, stops, focal ? bits.Fixed8() : null);
        }

        private static List<SwfLineStyle> LineStyles(SwfBits bits, int version)
        {
            int count = StyleCount(bits, version);
            var styles = new List<SwfLineStyle>(count);
            for (int i = 0; i < count; i++)
                styles.Add(version == 4 ? LineStyle2(bits) : new SwfLineStyle(bits.U16(), Color(bits, version)));
            return styles;
        }

        private static SwfLineStyle LineStyle2(SwfBits bits)
        {
            int width = bits.U16();
            int startCap = (int)bits.UB(2);
            int join = (int)bits.UB(2);
            bool hasFill = bits.Flag();
            bool noHScale = bits.Flag();
            bool noVScale = bits.Flag();
            bool pixelHinting = bits.Flag();
            bits.UB(5);
            bool noClose = bits.Flag();
            int endCap = (int)bits.UB(2);
            const int MiterJoin = 2;
            double? miterLimit = join == MiterJoin ? bits.Fixed8() : null;
            SwfColor? color = hasFill ? null : bits.Rgba();
            SwfFillStyle? fill = hasFill ? FillStyle(bits, 4) : null;
            return new SwfLineStyle(width, color, fill, (startCap, endCap), join, miterLimit, noHScale, noVScale, pixelHinting, noClose);
        }

        private static SwfColor Color(SwfBits bits, int version) => version >= 3 ? bits.Rgba() : bits.Rgb();
    }
}
