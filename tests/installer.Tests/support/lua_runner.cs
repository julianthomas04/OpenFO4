using AssetConverter.PapyrusLua.Lua;
using AssetConverter.PapyrusLua.Pex;
using MoonSharp.Interpreter;

namespace InstallerTests.Support
{
    /// <summary>Executes emitted Lua in MoonSharp (a managed Lua 5.2 interpreter, which matches the transpiler's
    /// "Lua 5.2+ for goto" target) so tests assert behaviour of the generated code, not just its text.</summary>
    internal sealed class LuaRunner
    {
        private readonly Script _script = new(CoreModules.Preset_SoftSandbox);
        private readonly Table _module;

        private LuaRunner(string luaSource)
        {
            _module = _script.DoString(luaSource).Table;
        }

        public static LuaRunner Load(PexFile pex) => new(LuaEmitter.Emit(pex));

        public static LuaRunner LoadSource(string luaSource) => new(luaSource);

        /// <summary>Emits a one-function script and loads it.</summary>
        public static LuaRunner LoadFunction(PexFunction function) => Load(PexModel.File(PexModel.Script("Test", function)));

        public Table Module => _module;

        /// <summary>Calls any Lua function value that belongs to this runner's script.</summary>
        public DynValue Invoke(DynValue function, params object[] args) => _script.Call(function, args);

        public Table NewObject()
        {
            var obj = new Table(_script);
            var meta = new Table(_script);
            meta["__index"] = _module;
            obj.MetaTable = meta;
            return obj;
        }

        public Table NewTable(params object[] items)
        {
            var table = new Table(_script);
            foreach (object item in items)
                table.Append(DynValue.FromObject(_script, item));
            return table;
        }

        /// <summary>Calls <c>module.name(args...)</c> without a self argument (global functions).</summary>
        public DynValue CallGlobal(string name, params object[] args) =>
            _script.Call(_module.Get(name), args);

        /// <summary>Calls <c>module.name(self, args...)</c>.</summary>
        public DynValue Call(Table self, string name, params object[] args) =>
            _script.Call(_module.Get(name), new object[] { self }.Concat(args).ToArray());

        public static void AssertCompiles(string luaSource) => new Script(CoreModules.Preset_SoftSandbox).LoadString(luaSource);
    }
}
