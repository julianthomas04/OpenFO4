using System.Globalization;

namespace AssetConverter.PapyrusLua.Lua;

public static class LuaNumberFormat
{
    /// <summary>Lua source for a float constant: always parses as a float, never culture dependent.</summary>
    public static string FloatLiteral(float v)
    {
        if (float.IsNaN(v)) return "(0/0)";
        if (float.IsPositiveInfinity(v)) return "math.huge";
        if (float.IsNegativeInfinity(v)) return "(-math.huge)";
        string s = v.ToString("R", CultureInfo.InvariantCulture);
        if (!s.Contains('.') && !s.Contains('e') && !s.Contains('E'))
            s += ".0";
        return s;
    }
}
