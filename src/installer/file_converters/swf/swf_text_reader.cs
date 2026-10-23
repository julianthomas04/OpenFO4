using System.Text;

namespace AssetConverter.Swf
{
    /// <summary>Reads the text characters of a SWF: DefineEditText, DefineText 1/2 and the font definitions that name their fonts.</summary>
    internal static class SwfTextReader
    {
        public static SwfEditText ReadEditText(SwfBits bits)
        {
            int id = bits.U16();
            TwipRect bounds = bits.Rect();
            bool hasText = bits.Flag();
            bool wordWrap = bits.Flag();
            bool multiline = bits.Flag();
            bool password = bits.Flag();
            bool readOnly = bits.Flag();
            bool hasTextColor = bits.Flag();
            bool hasMaxLength = bits.Flag();
            bool hasFont = bits.Flag();
            bool hasFontClass = bits.Flag();
            bool autoSize = bits.Flag();
            bool hasLayout = bits.Flag();
            bool noSelect = bits.Flag();
            bool border = bits.Flag();
            bool wasStatic = bits.Flag();
            bool html = bits.Flag();
            bool useOutlines = bits.Flag();

            int? fontId = hasFont ? bits.U16() : null;
            string? fontClass = hasFontClass ? bits.String() : null;
            int? fontHeight = hasFont || hasFontClass ? bits.U16() : null;
            SwfColor? color = hasTextColor ? bits.Rgba() : null;
            int? maxLength = hasMaxLength ? bits.U16() : null;
            SwfTextLayout? layout = null;
            if (hasLayout)
            {
                int align = bits.U8();
                if (align > (int)SwfTextAlign.Justify)
                    throw new SwfFormatException($"Text field {id} has alignment {align}, which is not a SWF alignment.");
                layout = new SwfTextLayout((SwfTextAlign)align, bits.U16(), bits.U16(), bits.U16(), bits.S16());
            }
            string variableName = bits.String();
            string? initialText = hasText ? bits.String() : null;
            var flags = new SwfEditTextFlags(wordWrap, multiline, password, readOnly, autoSize, noSelect, border, wasStatic, html, useOutlines);
            return new SwfEditText(id, bounds, flags, fontId, fontClass, fontHeight, color, maxLength, layout, variableName, initialText);
        }

        public static SwfStaticText ReadStaticText(SwfBits bits, bool withAlpha)
        {
            int id = bits.U16();
            TwipRect bounds = bits.Rect();
            SwfMatrix matrix = bits.Matrix();
            int glyphBits = bits.U8();
            int advanceBits = bits.U8();
            var runs = new List<SwfTextRun>();
            while (true)
            {
                bits.Align();
                bool isRecord = bits.Flag();
                if (!isRecord)
                {
                    bits.UB(7);
                    return new SwfStaticText(id, bounds, matrix, runs);
                }
                bits.UB(3);
                bool hasFont = bits.Flag();
                bool hasColor = bits.Flag();
                bool hasYOffset = bits.Flag();
                bool hasXOffset = bits.Flag();
                int? fontId = hasFont ? bits.U16() : null;
                SwfColor? color = hasColor ? (withAlpha ? bits.Rgba() : bits.Rgb()) : null;
                int? xOffset = hasXOffset ? bits.S16() : null;
                int? yOffset = hasYOffset ? bits.S16() : null;
                int? height = hasFont ? bits.U16() : null;
                int glyphCount = bits.U8();
                var glyphs = new List<SwfGlyphEntry>(glyphCount);
                for (int i = 0; i < glyphCount; i++)
                    glyphs.Add(new SwfGlyphEntry((int)bits.UB(glyphBits), bits.SB(advanceBits)));
                runs.Add(new SwfTextRun(fontId, color, xOffset, yOffset, height, glyphs));
            }
        }

        /// <summary>DefineFont2 or 3: the name, style, code table and the layout metrics when present.</summary>
        public static SwfFont ReadFont(SwfBits bits, int version)
        {
            int id = bits.U16();
            bool hasLayout = bits.Flag();
            bits.Flag();
            bits.Flag();
            bits.Flag();
            bool wideOffsets = bits.Flag();
            bool wideCodes = bits.Flag();
            bool italic = bits.Flag();
            bool bold = bits.Flag();
            bits.U8();
            string name = FontName(bits.Bytes(bits.U8()));
            int glyphCount = bits.U16();

            int offsetTableStart = bits.Position;
            int offsetSize = wideOffsets ? 4 : 2;
            var codes = new List<int>(glyphCount);
            int? ascent = null, descent = null, leading = null;
            if (glyphCount > 0)
            {
                bits.Skip(glyphCount * offsetSize);
                long codeTableOffset = wideOffsets ? bits.U32() : bits.U16();
                bits.Skip(checked((int)(offsetTableStart + codeTableOffset - bits.Position)));
                for (int i = 0; i < glyphCount; i++)
                    codes.Add(wideCodes ? bits.U16() : bits.U8());
            }
            else if (bits.Remaining >= offsetSize && (!hasLayout || bits.Remaining > EmptyLayoutSize))
            {
                bits.Skip(offsetSize);
            }
            if (hasLayout)
            {
                ascent = bits.U16();
                descent = bits.U16();
                leading = bits.S16();
            }
            const int Font2EmSize = 1024;
            const int Font3EmSize = 20480;
            return new SwfFont(id, version, name, bold, italic, version == 3 ? Font3EmSize : Font2EmSize, ascent, descent, leading, codes);
        }

        /// <summary>Ascent, descent, leading and the kerning count of a font layout without glyphs.</summary>
        private const int EmptyLayoutSize = 8;

        public static SwfFont ReadFont4(SwfBits bits)
        {
            int id = bits.U16();
            int flags = bits.U8();
            bool italic = (flags & 0x02) != 0;
            bool bold = (flags & 0x01) != 0;
            return new SwfFont(id, 4, bits.String(), bold, italic, 0, null, null, null, []);
        }

        private static string FontName(byte[] raw)
        {
            int end = Array.IndexOf(raw, (byte)0);
            return Encoding.UTF8.GetString(raw, 0, end < 0 ? raw.Length : end);
        }
    }
}
