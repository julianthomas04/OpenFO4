# Copyright 2026 Julian Thomas and OpenFO4 contributors
# SPDX-License-Identifier: Apache-2.0
"""Run only under orchestrator scheduling: synthetic exporter compatibility."""
import argparse
import json
from pathlib import Path
import subprocess
import sys
sys.path.insert(0, str(Path(__file__).resolve().parent))
from fixtures import create
from normalization import normalized


def invoke(java, jar, arguments, success=True):
    result = subprocess.run([str(java), "-Djava.awt.headless=true", "-Xmx1024m", "-jar", str(jar), *map(str, arguments)],
                            capture_output=True, text=True, timeout=120)
    if (result.returncode == 0) != success:
        raise AssertionError(f"Unexpected exit {result.returncode}: {arguments}\n{result.stdout}\n{result.stderr}")
    return result


def main():
    parser = argparse.ArgumentParser()
    parser.add_argument("--java", required=True, type=Path)
    parser.add_argument("--candidate", required=True, type=Path)
    parser.add_argument("--reference", required=True, type=Path)
    parser.add_argument("--work", required=True, type=Path)
    args = parser.parse_args()
    if args.work.exists():
        raise ValueError("Use a fresh test work directory")
    create(args.work / "inputs")
    help_text = invoke(args.java, args.candidate, ["-help"]).stdout
    if not help_text.startswith("JPEXS Free Flash Decompiler v.26.2.1-openfo4."):
        raise AssertionError("Version/cache contract failed")
    cases = [
        ("font", "exports.swf", ["-export", "font"], 1),
        ("unused-video-font", "unused-video.swf", ["-export", "font"], 1),
        ("visible-video-font", "visible-video.swf", ["-export", "font"], 1),
        ("frame", "exports.swf", ["-format", "frame:svg", "-ignorebackground", "-export", "frame"], 2),
        ("unused-video-frame", "unused-video.swf", ["-format", "frame:svg", "-ignorebackground", "-export", "frame"], 2),
        ("nested", "nested.swf", ["-format", "frame:svg", "-ignorebackground", "-sublength", "2", "-export", "frame"], 2),
        ("symbols", "exports.swf", ["-format", "sprite:svg,shape:svg", "-ignorebackground", "-selectid", "1,2", "-select", "1:1,2:1,2", "-export", "sprite,shape"], 3),
    ]
    records = []
    for name, filename, options, expected in cases:
        input_path = args.work / "inputs" / filename
        for tool, jar in (("reference", args.reference), ("candidate", args.candidate)):
            output = args.work / tool / name
            invoke(args.java, jar, [*options, output, input_path])
        reference = args.work / "reference" / name
        candidate = args.work / "candidate" / name
        left = {str(path.relative_to(reference)): normalized(path) for path in reference.rglob("*") if path.is_file()}
        right = {str(path.relative_to(candidate)): normalized(path) for path in candidate.rglob("*") if path.is_file()}
        if len(left) != expected or left != right:
            raise AssertionError(f"Export contract differs for {name}: expected {expected}; reference {list(left)}; candidate {list(right)}")
        records.append({"case": name, "files": sorted(left)})
    invoke(args.java, args.candidate, ["-unknown"], success=False)
    video_failure = invoke(args.java, args.candidate, ["-format", "frame:svg", "-ignorebackground", "-export", "frame",
                                                    args.work / "unsupported-video", args.work / "inputs/visible-video.swf"], success=False)
    if "unsupported" not in video_failure.stderr.lower():
        raise AssertionError("Visible unsupported video must fail with its cause")
    invoke(args.java, args.candidate, ["-export", "font", args.work / "bad", args.work / "inputs/malformed.swf"], success=False)
    invoke(args.java, args.candidate, ["-format", "sprite:svg,shape:svg", "-selectid", "1", "-select", "1:2", "-export", "sprite,shape",
                                     args.work / "bad-frame", args.work / "inputs/exports.swf"], success=False)
    report = {"status": "passed", "cases": records,
              "negativeChecks": ["unknown-option", "visible-video", "malformed", "shape-frame"]}
    (args.work / "results.json").write_text(json.dumps(report, indent=2) + "\n", encoding="utf-8")
    print(json.dumps(report, indent=2))


if __name__ == "__main__":
    main()
