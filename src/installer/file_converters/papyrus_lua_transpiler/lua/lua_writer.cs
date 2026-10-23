// Thin StringBuilder wrapper with indentation tracking.
// Used by LuaEmitter / (later) opcode_lowering.cs.

using System.Text;

namespace AssetConverter.PapyrusLua.Lua;

public sealed class LuaWriter
{
    readonly StringBuilder _sb = new();
    int _indent;
    const string IndentUnit = "    ";

    public void WriteLine(string line = "")
    {
        if (line.Length > 0)
            for (int i = 0; i < _indent; i++) _sb.Append(IndentUnit);
        _sb.Append(line);
        _sb.Append('\n');
    }

    public void Indent() => _indent++;
    public void Dedent()
    {
        if (_indent <= 0) throw new InvalidOperationException("Dedent below zero");
        _indent--;
    }

    public override string ToString() => _sb.ToString();
}
