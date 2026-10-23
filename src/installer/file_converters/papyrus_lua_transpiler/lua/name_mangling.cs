using System.Globalization;
// Identifier / string helpers for Lua emission.
//
// Papyrus identifiers can contain characters (`::`, spaces) that are not
// legal in Lua identifiers. We either keyword-escape / sanitize, or fall
// back to bracket-indexed string keys.

using System.Text;

namespace AssetConverter.PapyrusLua.Lua;

public static class NameMangling
{
    // Lua 5.1/5.2 reserved words. Shared between versions with the addition
    // of `goto` in 5.2 — we target 5.2+ (required for hoisted-local gotos),
    // so we treat all of these as reserved.
    static readonly HashSet<string> LuaKeywords = new(StringComparer.Ordinal)
    {
        "and", "break", "do", "else", "elseif", "end", "false", "for",
        "function", "goto", "if", "in", "local", "nil", "not", "or",
        "repeat", "return", "then", "true", "until", "while",
    };

    public static bool IsValidLuaIdentifier(string name)
    {
        if (string.IsNullOrEmpty(name)) return false;
        if (LuaKeywords.Contains(name)) return false;
        char c0 = name[0];
        if (!(char.IsAsciiLetter(c0) || c0 == '_')) return false;
        for (int i = 1; i < name.Length; i++)
        {
            char c = name[i];
            if (!(char.IsAsciiLetterOrDigit(c) || c == '_')) return false;
        }
        return true;
    }

    public static string ToLuaStringLiteral(string s)
    {
        var sb = new StringBuilder(s.Length + 2);
        sb.Append('"');
        foreach (char c in s)
        {
            switch (c)
            {
                case '\\': sb.Append("\\\\"); break;
                case '"': sb.Append("\\\""); break;
                case '\n': sb.Append("\\n"); break;
                case '\r': sb.Append("\\r"); break;
                case '\t': sb.Append("\\t"); break;
                default:
                    if (c < 0x20 || c == 0x7F) sb.Append(CultureInfo.InvariantCulture, $"\\{(int)c:D3}");
                    else sb.Append(c);
                    break;
            }
        }
        sb.Append('"');
        return sb.ToString();
    }

    // Rewrite an arbitrary Papyrus identifier so it becomes a legal Lua
    // local-variable / parameter name. Collisions across locals within
    // one function are vanishingly rare (Papyrus forbids duplicate locals
    // so the only failure mode is if sanitization maps two distinct names
    // to the same string — we ignore that for now).
    public static string ToLocalIdentifier(string id)
    {
        if (IsValidLuaIdentifier(id)) return id;
        var sb = new StringBuilder(id.Length);
        foreach (char c in id)
            sb.Append(char.IsAsciiLetterOrDigit(c) || c == '_' ? c : '_');
        string s = sb.Length == 0 ? "_" : sb.ToString();
        if (char.IsDigit(s[0])) s = "_" + s;
        if (LuaKeywords.Contains(s)) s = "_" + s;
        return s;
    }
}
