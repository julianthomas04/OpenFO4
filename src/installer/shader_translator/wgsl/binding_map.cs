using AssetConverter.ShaderTranslation.Dxbc;

namespace AssetConverter.ShaderTranslation.Wgsl
{
    public enum TextureOverride { None, Color, Depth }

    /// <summary>
    /// Where a DXBC register lives in WGSL. <see cref="UavFormat"/> (WGSL storage texel format such as <c>r32float</c>) is required
    /// for typed UAVs. <see cref="Name"/> must be a module-unique identifier (prefix it with the
    /// shader name). An <see cref="External"/> binding is not declared by the emitter: the wrapper declares a global of that
    /// name with a compatible type.
    /// </summary>
    public sealed record ResourceBinding(string Name, int Group, int Binding, bool External = false, TextureOverride TextureOverride = TextureOverride.None, string? UavFormat = null);

    /// <summary>Maps cbN / tN / sN / uN register numbers to WGSL bindings.</summary>
    public sealed class ShaderBindingMap
    {
        public IDictionary<int, ResourceBinding> ConstantBuffers { get; } = new Dictionary<int, ResourceBinding>();
        public IDictionary<int, ResourceBinding> Resources { get; } = new Dictionary<int, ResourceBinding>();
        public IDictionary<int, ResourceBinding> Samplers { get; } = new Dictionary<int, ResourceBinding>();
        public IDictionary<int, ResourceBinding> UnorderedAccess { get; } = new Dictionary<int, ResourceBinding>();

        /// <summary>Group 0, bindings numbered in declaration order (constant buffers, samplers, resources, UAVs).</summary>
        public static ShaderBindingMap Sequential(Sm5Program program, string namePrefix)
        {
            var map = new ShaderBindingMap();
            int next = 0;
            Add(map.ConstantBuffers, program, Opcode.DclConstantBuffer, "cb", namePrefix, ref next);
            Add(map.Samplers, program, Opcode.DclSampler, "s", namePrefix, ref next);
            foreach (var op in new[] { Opcode.DclResource, Opcode.DclResourceRaw, Opcode.DclResourceStructured })
                Add(map.Resources, program, op, "t", namePrefix, ref next);
            foreach (var op in new[] { Opcode.DclUavTyped, Opcode.DclUavRaw, Opcode.DclUavStructured })
                Add(map.UnorderedAccess, program, op, "u", namePrefix, ref next);
            return map;
        }

        private static void Add(IDictionary<int, ResourceBinding> target, Sm5Program program, Opcode opcode, string prefix, string namePrefix, ref int next)
        {
            foreach (var ins in program.Instructions.Where(i => i.Opcode == opcode))
            {
                int register = ins.Operands[0].ImmediateIndex0;
                target[register] = new ResourceBinding($"{namePrefix}_{prefix}{register}", 0, next++);
            }
        }
    }
}
