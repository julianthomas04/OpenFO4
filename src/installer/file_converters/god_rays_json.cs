using System.Drawing;
using System.Text.Json.Nodes;
using Mutagen.Bethesda.Fallout4;

namespace AssetConverter
{
    /// <summary>
    /// The GDRY DATA fields with snake_case names. Colours are <c>[r,g,b]</c> bytes (Mutagen scales the stored 0..1 floats);
    /// scales, intensity and phases are the stored floats.
    /// </summary>
    internal static class GodRaysJson
    {
        public static JsonObject Data(IGodRaysGetter record) => new()
        {
            ["back_color"] = Rgb(record.BackColor),
            ["forward_color"] = Rgb(record.ForwardColor),
            ["air_color"] = Rgb(record.AirColor),
            ["back_color_scale"] = record.BackColorScale,
            ["forward_color_scale"] = record.ForwardColorScale,
            ["air_color_scale"] = record.AirColorScale,
            ["intensity"] = record.Intensity,
            ["back_phase"] = record.BackPhase,
            ["forward_phase"] = record.ForwardPhase
        };

        private static JsonArray Rgb(Color color) => new(color.R, color.G, color.B);
    }
}
