namespace AssetConverter.ShaderTranslation.Dxbc
{
    /// <summary>SM4/SM5 opcode numbers (D3D10_SB_OPCODE_TYPE / D3D11_SB_OPCODE_TYPE).</summary>
    public enum Opcode : ushort
    {
        Add = 0, And, Break, Breakc, Call, Callc, Case, Continue, Continuec, Cut,
        Default, DerivRtx, DerivRty, Discard, Div, Dp2, Dp3, Dp4, Else, Emit,
        EmitThenCut, Endif, Endloop, Endswitch, Eq, Exp, Frc, Ftoi, Ftou, Ge,
        Iadd, If, Ieq, Ige, Ilt, Imad, Imax, Imin, Imul, Ine,
        Ineg, Ishl, Ishr, Itof, Label, Ld, LdMs, Log, Loop, Lt,
        Mad, Min, Max, CustomData, Mov, Movc, Mul, Ne, Nop, Not,
        Or, Resinfo, Ret, Retc, RoundNe, RoundNi, RoundPi, RoundZ, Rsq, Sample,
        SampleC, SampleCLz, SampleL, SampleD, SampleB, Sqrt, Switch, Sincos, Udiv, Ult,
        Uge, Umul, Umad, Umax, Umin, Ushr, Utof, Xor, DclResource, DclConstantBuffer,
        DclSampler, DclIndexRange, DclGsOutputPrimitiveTopology, DclGsInputPrimitive, DclMaxOutputVertexCount, DclInput, DclInputSgv, DclInputSiv, DclInputPs, DclInputPsSgv,
        DclInputPsSiv, DclOutput, DclOutputSgv, DclOutputSiv, DclTemps, DclIndexableTemp, DclGlobalFlags, Reserved0, Lod, Gather4,
        Samplepos, Sampleinfo, Reserved1, HsDecls, HsControlPointPhase, HsForkPhase, HsJoinPhase, EmitStream, CutStream, EmitThenCutStream,
        InterfaceCall, Bufinfo, DerivRtxCoarse, DerivRtxFine, DerivRtyCoarse, DerivRtyFine, Gather4C, Gather4Po, Gather4PoC, Rcp,
        F32ToF16, F16ToF32, Uaddc, Usubb, Countbits, FirstbitHi, FirstbitLo, FirstbitShi, Ubfe, Ibfe,
        Bfi, Bfrev, Swapc, DclStream, DclFunctionBody, DclFunctionTable, DclInterface, DclInputControlPointCount, DclOutputControlPointCount, DclTessDomain,
        DclTessPartitioning, DclTessOutputPrimitive, DclHsMaxTessfactor, DclHsForkPhaseInstanceCount, DclHsJoinPhaseInstanceCount, DclThreadGroup, DclUavTyped, DclUavRaw, DclUavStructured, DclTgsmRaw,
        DclTgsmStructured, DclResourceRaw, DclResourceStructured, LdUavTyped, StoreUavTyped, LdRaw, StoreRaw, LdStructured, StoreStructured, AtomicAnd,
        AtomicOr, AtomicXor, AtomicCmpStore, AtomicIadd, AtomicImax, AtomicImin, AtomicUmax, AtomicUmin, ImmAtomicAlloc, ImmAtomicConsume,
        ImmAtomicIadd, ImmAtomicAnd, ImmAtomicOr, ImmAtomicXor, ImmAtomicExch, ImmAtomicCmpExch, ImmAtomicImax, ImmAtomicImin, ImmAtomicUmax, ImmAtomicUmin,
        Sync, Dadd, Dmax, Dmin, Dmul, Deq, Dge, Dlt, Dne, Dmov,
        Dmovc, Dtof, Ftod, EvalSnapped, EvalSampleIndex, EvalCentroid, DclGsInstanceCount, Abort, DebugBreak, Ddiv,
        Dfma, Drcp, Msad, Dtoi, Dtou, Itod, Utod,
    }

    public static class OpcodeInfo
    {
        private static readonly string[] Names =
        [
            "add", "and", "break", "breakc", "call", "callc", "case", "continue", "continuec", "cut",
            "default", "deriv_rtx", "deriv_rty", "discard", "div", "dp2", "dp3", "dp4", "else", "emit",
            "emitThenCut", "endif", "endloop", "endswitch", "eq", "exp", "frc", "ftoi", "ftou", "ge",
            "iadd", "if", "ieq", "ige", "ilt", "imad", "imax", "imin", "imul", "ine",
            "ineg", "ishl", "ishr", "itof", "label", "ld", "ld_ms", "log", "loop", "lt",
            "mad", "min", "max", "customdata", "mov", "movc", "mul", "ne", "nop", "not",
            "or", "resinfo", "ret", "retc", "round_ne", "round_ni", "round_pi", "round_z", "rsq", "sample",
            "sample_c", "sample_c_lz", "sample_l", "sample_d", "sample_b", "sqrt", "switch", "sincos", "udiv", "ult",
            "uge", "umul", "umad", "umax", "umin", "ushr", "utof", "xor", "dcl_resource", "dcl_constantBuffer",
            "dcl_sampler", "dcl_index_range", "dcl_gs_output_primitive_topology", "dcl_gs_input_primitive", "dcl_max_output_vertex_count", "dcl_input", "dcl_input_sgv", "dcl_input_siv", "dcl_input_ps", "dcl_input_ps_sgv",
            "dcl_input_ps_siv", "dcl_output", "dcl_output_sgv", "dcl_output_siv", "dcl_temps", "dcl_indexableTemp", "dcl_globalFlags", "reserved0", "lod", "gather4",
            "samplepos", "sampleinfo", "reserved1", "hs_decls", "hs_control_point_phase", "hs_fork_phase", "hs_join_phase", "emit_stream", "cut_stream", "emitThenCut_stream",
            "interface_call", "bufinfo", "deriv_rtx_coarse", "deriv_rtx_fine", "deriv_rty_coarse", "deriv_rty_fine", "gather4_c", "gather4_po", "gather4_po_c", "rcp",
            "f32tof16", "f16tof32", "uaddc", "usubb", "countbits", "firstbit_hi", "firstbit_lo", "firstbit_shi", "ubfe", "ibfe",
            "bfi", "bfrev", "swapc", "dcl_stream", "dcl_function_body", "dcl_function_table", "dcl_interface", "dcl_input_control_point_count", "dcl_output_control_point_count", "dcl_tessellator_domain",
            "dcl_tessellator_partitioning", "dcl_tessellator_output_primitive", "dcl_hs_max_tessfactor", "dcl_hs_fork_phase_instance_count", "dcl_hs_join_phase_instance_count", "dcl_thread_group", "dcl_uav_typed", "dcl_uav_raw", "dcl_uav_structured", "dcl_tgsm_raw",
            "dcl_tgsm_structured", "dcl_resource_raw", "dcl_resource_structured", "ld_uav_typed", "store_uav_typed", "ld_raw", "store_raw", "ld_structured", "store_structured", "atomic_and",
            "atomic_or", "atomic_xor", "atomic_cmp_store", "atomic_iadd", "atomic_imax", "atomic_imin", "atomic_umax", "atomic_umin", "imm_atomic_alloc", "imm_atomic_consume",
            "imm_atomic_iadd", "imm_atomic_and", "imm_atomic_or", "imm_atomic_xor", "imm_atomic_exch", "imm_atomic_cmp_exch", "imm_atomic_imax", "imm_atomic_imin", "imm_atomic_umax", "imm_atomic_umin",
            "sync", "dadd", "dmax", "dmin", "dmul", "deq", "dge", "dlt", "dne", "dmov",
            "dmovc", "dtof", "ftod", "eval_snapped", "eval_sample_index", "eval_centroid", "dcl_gs_instance_count", "abort", "debug_break", "ddiv",
            "dfma", "drcp", "msad", "dtoi", "dtou", "itod", "utod",
        ];

        public static int Count => Names.Length;

        public static bool IsKnown(uint opcode) => opcode < Names.Length;

        public static string Name(Opcode opcode) => Names[(int)opcode];

        public static bool IsDeclaration(Opcode op) =>
            op is >= Opcode.DclResource and <= Opcode.DclGlobalFlags
                or >= Opcode.DclStream and <= Opcode.DclTgsmStructured
                or Opcode.DclResourceRaw or Opcode.DclResourceStructured
                or Opcode.DclGsInstanceCount or Opcode.HsDecls or Opcode.HsControlPointPhase
                or Opcode.HsForkPhase or Opcode.HsJoinPhase;

        private static readonly Dictionary<Opcode, int> OperandCounts = BuildOperandCounts();

        private static Dictionary<Opcode, int> BuildOperandCounts()
        {
            var counts = new Dictionary<Opcode, int>();
            void Set(int count, params Opcode[] ops)
            {
                foreach (var op in ops) counts[op] = count;
            }

            Set(0, Opcode.Break, Opcode.Continue, Opcode.Default, Opcode.Else, Opcode.Endif, Opcode.Endloop, Opcode.Endswitch,
                Opcode.Loop, Opcode.Nop, Opcode.Ret, Opcode.Sync);
            Set(1, Opcode.Breakc, Opcode.Continuec, Opcode.Case, Opcode.Discard, Opcode.If, Opcode.Retc, Opcode.Switch);
            Set(2, Opcode.Mov, Opcode.Sqrt, Opcode.Rsq, Opcode.Rcp, Opcode.Exp, Opcode.Log, Opcode.Frc, Opcode.RoundNe, Opcode.RoundNi,
                Opcode.RoundPi, Opcode.RoundZ, Opcode.DerivRtx, Opcode.DerivRty, Opcode.DerivRtxCoarse, Opcode.DerivRtxFine,
                Opcode.DerivRtyCoarse, Opcode.DerivRtyFine, Opcode.Ftoi, Opcode.Ftou, Opcode.Itof, Opcode.Utof, Opcode.Ineg, Opcode.Not,
                Opcode.Countbits, Opcode.FirstbitHi, Opcode.FirstbitLo, Opcode.FirstbitShi, Opcode.Bfrev, Opcode.F32ToF16,
                Opcode.F16ToF32, Opcode.Bufinfo);
            Set(3, Opcode.Add, Opcode.Mul, Opcode.Div, Opcode.Dp2, Opcode.Dp3, Opcode.Dp4, Opcode.Min, Opcode.Max, Opcode.Eq, Opcode.Ne,
                Opcode.Lt, Opcode.Ge, Opcode.Ieq, Opcode.Ine, Opcode.Ilt, Opcode.Ige, Opcode.Ult, Opcode.Uge, Opcode.And, Opcode.Or,
                Opcode.Xor, Opcode.Iadd, Opcode.Imax, Opcode.Imin, Opcode.Umax, Opcode.Umin, Opcode.Ishl, Opcode.Ishr, Opcode.Ushr,
                Opcode.Sincos, Opcode.Ld, Opcode.Resinfo, Opcode.LdUavTyped, Opcode.StoreUavTyped, Opcode.LdRaw, Opcode.StoreRaw,
                Opcode.AtomicAnd, Opcode.AtomicOr, Opcode.AtomicXor, Opcode.AtomicIadd, Opcode.AtomicImax, Opcode.AtomicImin,
                Opcode.AtomicUmax, Opcode.AtomicUmin);
            Set(4, Opcode.Mad, Opcode.Movc, Opcode.Imad, Opcode.Umad, Opcode.Imul, Opcode.Umul, Opcode.Udiv, Opcode.Ubfe, Opcode.Ibfe,
                Opcode.LdMs, Opcode.LdStructured, Opcode.StoreStructured, Opcode.Sample, Opcode.Gather4, Opcode.AtomicCmpStore,
                Opcode.ImmAtomicIadd, Opcode.ImmAtomicAnd, Opcode.ImmAtomicOr, Opcode.ImmAtomicXor, Opcode.ImmAtomicExch,
                Opcode.ImmAtomicImax, Opcode.ImmAtomicImin, Opcode.ImmAtomicUmax, Opcode.ImmAtomicUmin);
            Set(5, Opcode.Bfi, Opcode.SampleB, Opcode.SampleL, Opcode.SampleC, Opcode.SampleCLz, Opcode.Gather4C, Opcode.ImmAtomicCmpExch);
            Set(6, Opcode.SampleD);
            return counts;
        }

        /// <summary>Exact operand count of an executable instruction the translator understands; null where it is not tabulated.</summary>
        public static int? ExpectedOperandCount(Opcode op) => OperandCounts.TryGetValue(op, out int count) ? count : null;

        /// <summary>Number of leading operands that are written by the instruction.</summary>
        public static int DestinationCount(Opcode op)
        {
            switch (op)
            {
                case Opcode.Sincos:
                case Opcode.Udiv:
                case Opcode.Umul:
                case Opcode.Imul:
                case Opcode.Uaddc:
                case Opcode.Usubb:
                case Opcode.Swapc:
                    return 2;
                case Opcode.Break:
                case Opcode.Breakc:
                case Opcode.Call:
                case Opcode.Callc:
                case Opcode.Case:
                case Opcode.Continue:
                case Opcode.Continuec:
                case Opcode.Cut:
                case Opcode.Default:
                case Opcode.Else:
                case Opcode.Emit:
                case Opcode.EmitThenCut:
                case Opcode.Endif:
                case Opcode.Endloop:
                case Opcode.Endswitch:
                case Opcode.If:
                case Opcode.Label:
                case Opcode.Loop:
                case Opcode.Nop:
                case Opcode.Ret:
                case Opcode.Retc:
                case Opcode.Switch:
                case Opcode.Discard:
                case Opcode.Sync:
                case Opcode.EmitStream:
                case Opcode.CutStream:
                case Opcode.EmitThenCutStream:
                case Opcode.InterfaceCall:
                case Opcode.Abort:
                case Opcode.DebugBreak:
                case Opcode.AtomicAnd:
                case Opcode.AtomicOr:
                case Opcode.AtomicXor:
                case Opcode.AtomicCmpStore:
                case Opcode.AtomicIadd:
                case Opcode.AtomicImax:
                case Opcode.AtomicImin:
                case Opcode.AtomicUmax:
                case Opcode.AtomicUmin:
                case Opcode.CustomData:
                    return 0;
                default:
                    return IsDeclaration(op) ? 0 : 1;
            }
        }
    }
}
