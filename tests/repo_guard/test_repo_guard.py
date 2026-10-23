"""Behaviour of tools/repo_guard: which files it flags, what the allow-list can waive and how the command line reports it."""

import base64
import os
import struct
import subprocess
import sys
import tempfile
import unittest
from pathlib import Path

REPO_ROOT = Path(__file__).resolve().parents[2]
GUARD_DIRECTORY = REPO_ROOT / "tools" / "repo_guard"
sys.path.insert(0, str(GUARD_DIRECTORY))
import byte_rules  # noqa: E402
import scanner  # noqa: E402
import text_rules  # noqa: E402
from allowlist import AllowEntry, AllowlistError, glob_to_regex, load_allowlist  # noqa: E402

PAD = bytes(64)
JUSTIFICATION = "Synthetic fixture used by the guard's own tests."

SIGNATURE_SAMPLES = {
    "DXBC shader container": b"DXBC" + PAD,
    "BA2 archive": b"BTDX" + PAD,
    "BSA archive": b"BSA\0" + PAD,
    "plugin / master (TES4)": b"TES4" + PAD,
    "Gamebryo NIF model": b"Gamebryo File Format, Version 20.2.0.7\n" + PAD,
    "DDS texture": b"DDS " + PAD,
    "Havok packfile": bytes([0x57, 0xE0, 0xE0, 0x57, 0x10, 0xC0, 0xC0, 0x10]) + PAD,
    "Havok tagfile": b"\0\0\0\x20TAG0" + PAD,
    "compiled Papyrus script (PEX)": struct.pack("<I", 0xFA57C0DE) + PAD,
    "xWMA audio": b"RIFF\x10\0\0\0XWMA" + PAD,
    "FUZ voice file": b"FUZE" + PAD,
    "BGSM material": b"BGSM" + PAD,
    "SWF movie (uncompressed)": b"FWS\x0b" + PAD,
    "Scaleform GFX movie": b"GFX\x09" + PAD,
}

TRANSLATED_SHADER = """\
struct ps_main_in {
    v0: vec4<f32>, // TEXCOORD0 mask=0x3
}
struct ps_main_out {
    o0: vec4<f32>, // SV_Target0 mask=0xf
}
fn ps_main(i: ps_main_in) -> ps_main_out {
    var o: ps_main_out;
    var r0: vec4<f32>;
    return o;
}
"""

HAND_WRITTEN_WRAPPER = """\
@fragment
fn fs_main(in: VertexOut) -> @location(0) vec4<f32> {
    var input: water_ps_in;
    input.v0 = in.uv;
    let result = water_ps(input);
    return result.o0;
}
"""


def dxbc_container(size: int = 64, version: int = 1, chunks: int = 2) -> bytes:
    header = b"DXBC" + bytes(16) + struct.pack("<III", version, size, chunks)
    return header + bytes(size - len(header))


class GuardTestCase(unittest.TestCase):
    def setUp(self) -> None:
        directory = tempfile.TemporaryDirectory()
        self.addCleanup(directory.cleanup)
        self.root = Path(directory.name)

    def write(self, relative_path: str, content: bytes | str) -> None:
        path = self.root / relative_path
        path.parent.mkdir(parents=True, exist_ok=True)
        path.write_bytes(content if isinstance(content, bytes) else content.encode("utf-8"))

    def scan(self, allowlist: list[AllowEntry] | None = None) -> scanner.ScanResult:
        return scanner.scan(self.root, scanner.list_directory_files(self.root), allowlist or [])

    def rules(self, allowlist: list[AllowEntry] | None = None) -> set[str]:
        return {finding.rule for finding in self.scan(allowlist).findings}

    def flagged_rules(self, relative_path: str, content: bytes | str) -> set[str]:
        self.write(relative_path, content)
        return self.rules()


class PathRuleTests(GuardTestCase):
    def test_game_formats_are_flagged_by_extension_regardless_of_case(self):
        for name in ("Meshes/rock.NIF", "textures/wall.dds", "Data/Fallout4.esm", "scripts/quest.pex", "ui/hud.swf", "a/b.psc"):
            with self.subTest(name=name):
                self.assertIn("forbidden-extension", self.flagged_rules(name, "x"))

    def test_converted_asset_formats_are_flagged(self):
        for name in ("rock.glb", "font.ttf", "walk.anims"):
            with self.subTest(name=name):
                self.assertIn("forbidden-extension", self.flagged_rules(name, "x"))

    def test_installer_output_directories_are_flagged(self):
        for name in ("install/cache/physics/blob.bin", "out/translated/a.txt", "x/Converted/y.txt"):
            with self.subTest(name=name):
                self.assertIn("forbidden-directory", self.flagged_rules(name, "x"))

    def test_a_file_named_like_an_output_directory_is_not_flagged(self):
        self.assertEqual(set(), self.flagged_rules("src/cache.rs", "fn main() {}\n"))

    def test_ordinary_source_files_pass(self):
        self.assertEqual(set(), self.flagged_rules("src/lib.rs", "pub fn add(a: u32, b: u32) -> u32 { a + b }\n"))


class ByteRuleTests(GuardTestCase):
    def test_every_game_format_signature_is_detected(self):
        for label, sample in SIGNATURE_SAMPLES.items():
            with self.subTest(label=label):
                findings = byte_rules.check_signatures("sample.bin", sample)
                self.assertEqual(1, len(findings))
                self.assertTrue(findings[0].detail.endswith(label), findings[0].detail)

    def test_signature_is_detected_in_a_file_with_an_innocent_name(self):
        self.assertIn("game-magic", self.flagged_rules("data/blob.dat", SIGNATURE_SAMPLES["BA2 archive"]))

    def test_text_that_merely_starts_with_a_magic_word_is_not_a_game_file(self):
        self.assertNotIn("game-magic", self.flagged_rules("notes.txt", "DDS texture notes\nTES4 record layout\n"))

    def test_swf_signature_requires_a_plausible_version_byte(self):
        self.assertNotIn("game-magic", self.flagged_rules("unrelated.bin", b"FWS\xff" + PAD))

    def test_xwma_requires_both_riff_and_xwma_markers(self):
        self.assertNotIn("game-magic", self.flagged_rules("plain.wav", b"RIFF\x10\0\0\0WAVE" + PAD))

    def test_dxbc_container_inside_another_file_is_detected(self):
        self.assertIn("embedded-dxbc", self.flagged_rules("pack.bin", b"\x44\x33\x22\x11" + bytes(40) + dxbc_container()))

    def test_dxbc_magic_without_a_valid_container_header_is_ignored(self):
        content = b"\0" * 8 + b"DXBC" + bytes(16) + struct.pack("<III", 2, 64, 2) + bytes(40)
        self.assertNotIn("embedded-dxbc", self.flagged_rules("noise.bin", content))

    def test_dxbc_container_larger_than_the_file_is_ignored(self):
        self.assertNotIn("embedded-dxbc", self.flagged_rules("truncated.bin", b"\0" * 8 + dxbc_container()[:40]))

    def test_dxbc_container_at_offset_zero_is_reported_once_as_a_signature(self):
        rules = self.flagged_rules("shader.bin", dxbc_container())
        self.assertIn("game-magic", rules)
        self.assertNotIn("embedded-dxbc", rules)

    def test_dxbc_string_in_source_code_is_not_a_container(self):
        self.assertEqual(set(), self.flagged_rules("parser.cs", 'static ReadOnlySpan<byte> DxbcMagic => "DXBC"u8;\n'))


class BinaryFileTests(GuardTestCase):


    def test_unknown_binary_file_is_flagged(self):
        self.assertIn("unexpected-binary", self.flagged_rules("tool.exe", b"MZ" + PAD))

    def test_text_file_is_not_binary(self):
        self.assertNotIn("unexpected-binary", self.flagged_rules("readme.txt", "plain text\n"))

    def test_oversized_text_file_is_flagged(self):
        self.assertIn("oversized-text", self.flagged_rules("big.txt", "line of ordinary prose\n" * (scanner.MAX_TEXT_BYTES // 20)))

    def test_symlink_is_flagged_without_following_it(self):
        self.write("target.txt", "x")
        try:
            os.symlink(self.root / "target.txt", self.root / "link.txt")
        except (OSError, NotImplementedError):
            self.skipTest("symbolic links are not available in this environment")
        self.assertIn("symlink", self.rules())


class TranslatedShaderTests(GuardTestCase):
    def test_translator_output_is_flagged(self):
        self.assertIn("translated-wgsl", self.flagged_rules("shader.wgsl", TRANSLATED_SHADER))

    def test_translator_output_is_flagged_in_files_that_are_not_wgsl(self):
        self.assertIn("translated-wgsl", self.flagged_rules("embedded.rs", f'const SHADER: &str = r#"{TRANSLATED_SHADER}"#;\n'))

    def test_each_decisive_fingerprint_is_enough(self):
        for snippet in (
            "var r3: vec4<f32>;",
            "var discard_pending: bool = false;",
            "var bits_3f800000: u32 = 0x3f800000u;",
            "    o0: vec4<f32>, // SV_Target0 mask=0xf",
        ):
            with self.subTest(snippet=snippet):
                self.assertIn("translated-wgsl", self.flagged_rules("one.wgsl", f"fn helper() {{\n{snippet}\n}}\n"))

    def test_two_weak_fingerprints_are_flagged_but_one_is_not(self):
        struct_only = "struct a_in { v0: vec4<f32>, }\n"
        self.assertNotIn("translated-wgsl", self.flagged_rules("weak.wgsl", struct_only))
        self.assertIn("translated-wgsl", self.flagged_rules("weak.wgsl", struct_only + "let tmp0 = 1.0;\n"))

    def test_many_dxbc_style_temporaries_are_flagged_even_when_renamed(self):
        body = "\n".join(f"    a = r{index} + b;" for index in range(10))
        self.assertIn("translated-wgsl", self.flagged_rules("renamed.wgsl", f"fn f() {{\n{body}\n}}\n"))

    def test_hand_written_wrapper_that_calls_a_translated_function_passes(self):
        self.assertEqual(set(), self.flagged_rules("wrapper.wgsl", HAND_WRITTEN_WRAPPER))

    def test_a_few_r_numbered_variables_pass(self):
        body = "\n".join(f"    let r{index} = {index}.0;" for index in range(7))
        self.assertEqual(set(), self.flagged_rules("small.wgsl", f"fn f() {{\n{body}\n}}\n"))

    def test_finding_reports_the_line_of_the_first_fingerprint(self):
        self.write("late.wgsl", "// header\n\n" + TRANSLATED_SHADER)
        finding = next(f for f in self.scan().findings if f.rule == "translated-wgsl")
        self.assertEqual(3, finding.line)


class ShaderDisassemblyTests(GuardTestCase):
    def test_disassembly_listing_is_flagged(self):
        listing = "ps_5_0\n    dcl_globalFlags refactoringAllowed\n    dcl_input_ps linear v1.xy\n    ret\n"
        self.assertIn("dxbc-disassembly", self.flagged_rules("listing.txt", listing))

    def test_windows_line_endings_do_not_hide_a_listing(self):
        self.assertIn("dxbc-disassembly", self.flagged_rules("listing.txt", "ps_5_0\r\n    ret\r\n"))

    def test_compiler_banner_is_flagged(self):
        self.assertIn("dxbc-disassembly", self.flagged_rules("out.txt", "//\n// Generated by Microsoft (R) HLSL Shader Compiler 10.1\n"))

    def test_prose_and_string_literals_mentioning_instructions_pass(self):
        self.assertEqual(set(), self.flagged_rules("doc.md", 'The decoder reads `dcl_temps` and "dcl_input" declarations.\n'))


class BulkDataTests(GuardTestCase):
    def test_base64_blob_is_flagged(self):
        blob = base64.b64encode(bytes(range(256)) * 2).decode()
        self.assertIn("encoded-blob", self.flagged_rules("data.rs", f'const DATA: &str = "{blob}";\n'))

    def test_hex_dump_without_separators_is_flagged(self):
        self.assertIn("encoded-blob", self.flagged_rules("dump.txt", bytes(range(100)).hex()))

    def test_hash_sized_hex_strings_pass(self):
        self.assertEqual(set(), self.flagged_rules("Cargo.lock", 'checksum = "' + "ab12" * 16 + '"\n'))

    def test_hex_byte_array_is_flagged(self):
        table = ", ".join(f"0x{value:02x}" for value in range(text_rules.MIN_HEX_BYTES))
        self.assertIn("hex-byte-table", self.flagged_rules("bytes.rs", f"static T: [u8; 64] = [{table}];\n"))

    def test_hex_byte_array_below_the_threshold_passes(self):
        table = ", ".join(f"0x{value:02x}" for value in range(text_rules.MIN_HEX_BYTES - 1))
        self.assertNotIn("hex-byte-table", self.flagged_rules("bytes.rs", f"static T: [u8; 63] = [{table}];\n"))

    def test_numeric_table_is_flagged_across_lines(self):
        rows = "\n".join(f"    {value}.25, {value}.5," for value in range(text_rules.MIN_NUMBER_LITERALS // 2))
        self.assertIn("numeric-table", self.flagged_rules("lut.rs", f"const LUT: [f32; N] = [\n{rows}\n];\n"))

    def test_numeric_table_below_the_threshold_passes(self):
        values = ", ".join(f"{value}.5" for value in range(text_rules.MIN_NUMBER_LITERALS - 1))
        self.assertNotIn("numeric-table", self.flagged_rules("lut.rs", f"const LUT: [f32; 127] = [{values}];\n"))



    def test_numbers_interleaved_with_code_are_not_a_table(self):
        lines = "\n".join(f"let v{index} = {index};" for index in range(text_rules.MIN_NUMBER_LITERALS * 2))
        self.assertNotIn("numeric-table", self.flagged_rules("consts.rs", lines + "\n"))


class AllowlistMatchingTests(unittest.TestCase):
    def test_star_stays_inside_a_path_segment(self):
        pattern = glob_to_regex("src/*.rs")
        self.assertTrue(pattern.match("src/lib.rs"))
        self.assertFalse(pattern.match("src/deep/lib.rs"))

    def test_double_star_crosses_segments_and_may_match_none(self):
        pattern = glob_to_regex("third_party/**/license.txt")
        self.assertTrue(pattern.match("third_party/a/b/license.txt"))
        self.assertTrue(pattern.match("third_party/license.txt"))
        self.assertTrue(glob_to_regex("third_party/**").match("third_party/a/b.jar"))
        self.assertFalse(glob_to_regex("third_party/**").match("other/third_party/a.jar"))

    def test_regex_metacharacters_in_paths_are_literal(self):
        pattern = glob_to_regex("a+b(1).txt")
        self.assertTrue(pattern.match("a+b(1).txt"))
        self.assertFalse(pattern.match("aab(1).txt"))

    def test_question_mark_matches_exactly_one_non_separator(self):
        pattern = glob_to_regex("v?.txt")
        self.assertTrue(pattern.match("v1.txt"))
        self.assertFalse(pattern.match("v/.txt"))
        self.assertFalse(pattern.match("v12.txt"))


class AllowlistScanTests(GuardTestCase):
    @staticmethod
    def entry(paths: list[str], rules: list[str]) -> AllowEntry:
        return AllowEntry(tuple(paths), frozenset(rules), JUSTIFICATION)

    def test_covered_finding_is_waived_and_counted(self):
        self.write("tool.exe", b"MZ" + PAD)
        result = self.scan([self.entry(["tool.exe"], ["unexpected-binary"])])
        self.assertTrue(result.passed)
        self.assertEqual(1, result.waived)

    def test_waiver_is_limited_to_the_listed_rule(self):
        self.write("pack.bin", b"BTDX" + PAD)
        result = self.scan([self.entry(["pack.bin"], ["unexpected-binary"])])
        self.assertEqual({"game-magic"}, {finding.rule for finding in result.findings})

    def test_waiver_is_limited_to_the_listed_paths(self):
        self.write("a/tool.exe", b"MZ" + PAD)
        self.write("b/tool.exe", b"MZ" + PAD)
        result = self.scan([self.entry(["a/**"], ["unexpected-binary"])])
        self.assertEqual(["b/tool.exe"], [finding.path for finding in result.findings])

    def test_entry_that_waives_nothing_fails_the_scan(self):
        self.write("fine.txt", "x")
        result = self.scan([self.entry(["gone/**"], ["unexpected-binary"])])
        self.assertFalse(result.passed)
        self.assertEqual(1, len(result.unused_entries))

    def test_unused_rule_of_a_partly_used_entry_is_reported(self):
        self.write("tool.exe", b"MZ" + PAD)
        result = self.scan([self.entry(["tool.exe"], ["unexpected-binary", "game-magic"])])
        self.assertFalse(result.passed)
        self.assertEqual(frozenset({"game-magic"}), result.unused_entries[0].unused_rules())


class AllowlistFileTests(GuardTestCase):
    def load(self, content: str) -> list[AllowEntry]:
        path = self.root / "allow.toml"
        path.write_text(content, encoding="utf-8")
        return load_allowlist(path, scanner.RULE_IDS)

    def test_valid_entries_are_loaded(self):
        entries = self.load(f'[[allow]]\npaths = ["a/**"]\nrules = ["game-magic"]\njustification = "{JUSTIFICATION}"\n')
        self.assertEqual(("a/**",), entries[0].patterns)
        self.assertEqual(frozenset({"game-magic"}), entries[0].rules)

    def test_empty_file_means_no_exceptions(self):
        self.assertEqual([], self.load(""))

    def test_malformed_entries_are_rejected(self):
        valid = f'paths = ["a"]\nrules = ["game-magic"]\njustification = "{JUSTIFICATION}"\n'
        cases = {
            "missing justification": 'paths = ["a"]\nrules = ["game-magic"]\n',
            "short justification": 'paths = ["a"]\nrules = ["game-magic"]\njustification = "ok"\n',
            "unknown rule": 'paths = ["a"]\nrules = ["nonsense"]\n' + f'justification = "{JUSTIFICATION}"\n',
            "empty paths": 'paths = []\nrules = ["game-magic"]\n' + f'justification = "{JUSTIFICATION}"\n',
            "extra key": valid + "extra = 1\n",
        }
        for name, body in cases.items():
            with self.subTest(name=name), self.assertRaises(AllowlistError):
                self.load("[[allow]]\n" + body)

    def test_unknown_top_level_table_is_rejected(self):
        with self.assertRaises(AllowlistError):
            self.load("[other]\nkey = 1\n")

    def test_invalid_toml_and_missing_file_are_rejected(self):
        with self.assertRaises(AllowlistError):
            self.load("[[allow\n")
        with self.assertRaises(AllowlistError):
            load_allowlist(self.root / "missing.toml", scanner.RULE_IDS)


class GitListingTests(GuardTestCase):
    def git(self, *arguments: str) -> None:
        subprocess.run(["git", "-C", str(self.root), *arguments], check=True, capture_output=True)

    def setUp(self) -> None:
        super().setUp()
        try:
            self.git("init", "-q")
        except (FileNotFoundError, subprocess.CalledProcessError):
            self.skipTest("git is not available")

    def test_lists_tracked_and_untracked_files_but_not_ignored_or_deleted_ones(self):
        self.write(".gitignore", "ignored.txt\n")
        self.write("tracked.txt", "x")
        self.write("deleted.txt", "x")
        self.git("add", "tracked.txt", "deleted.txt")
        (self.root / "deleted.txt").unlink()
        self.write("untracked.txt", "x")
        self.write("ignored.txt", "x")

        self.assertEqual([".gitignore", "tracked.txt", "untracked.txt"], scanner.list_git_files(self.root))

    def test_directory_that_is_not_a_repository_is_an_error(self):
        with tempfile.TemporaryDirectory() as plain, self.assertRaises(scanner.GuardError):
            scanner.list_git_files(Path(plain))


class CommandLineTests(GuardTestCase):
    def run_guard(self, allowlist_content: str | None = "") -> subprocess.CompletedProcess[str]:
        allowlist = self.root.parent / f"{self.root.name}.toml"
        self.addCleanup(lambda: allowlist.unlink(missing_ok=True))
        if allowlist_content is not None:
            allowlist.write_text(allowlist_content, encoding="utf-8")
        return subprocess.run(
            [sys.executable, str(GUARD_DIRECTORY), "--root", str(self.root), "--allowlist", str(allowlist), "--no-git"],
            capture_output=True, text=True, check=False,
        )

    def test_clean_tree_exits_zero(self):
        self.write("main.rs", "fn main() {}\n")
        result = self.run_guard()
        self.assertEqual(0, result.returncode, result.stdout + result.stderr)

    def test_finding_exits_one_and_names_file_rule_and_line(self):
        self.write("src/shader.wgsl", "// x\n" + TRANSLATED_SHADER)
        result = self.run_guard()
        self.assertEqual(1, result.returncode)
        self.assertIn("[translated-wgsl]", result.stdout)
        self.assertIn("src/shader.wgsl:2", result.stdout)

    def test_stale_allowlist_entry_exits_one(self):
        self.write("main.rs", "fn main() {}\n")
        result = self.run_guard(f'[[allow]]\npaths = ["x"]\nrules = ["game-magic"]\njustification = "{JUSTIFICATION}"\n')
        self.assertEqual(1, result.returncode)
        self.assertIn("stale-allowlist-entry", result.stdout)

    def test_unreadable_allowlist_exits_two(self):
        result = self.run_guard(allowlist_content=None)
        self.assertEqual(2, result.returncode)
        self.assertIn("cannot read allow-list", result.stderr)


if __name__ == "__main__":
    unittest.main()
