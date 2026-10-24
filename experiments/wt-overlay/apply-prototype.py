"""Apply only the isolated native test primitive to a pinned Terminal checkout."""
import pathlib
import shutil
import subprocess
import sys

source = pathlib.Path(sys.argv[1]).resolve()
expected = "7ff11ee107c984e236960ec948b450ccbc10187b"
actual = subprocess.check_output(["git", "-C", str(source), "rev-parse", "HEAD"], text=True).strip()
if actual != expected:
    raise SystemExit(f"Terminal source mismatch: {actual}")
base = pathlib.Path(__file__).resolve().parent
output = source / "src/buffer/out"
for src, dst in [("NativeOverlaySnapshot.hpp", output / "NativeOverlaySnapshot.hpp"),
                 ("NativeOverlayTests.cpp", output / "ut_textbuffer/NativeOverlayTests.cpp")]:
    if dst.exists():
        raise SystemExit(f"Refusing to overwrite {dst}")
    shutil.copyfile(base / src, dst)
project = output / "ut_textbuffer/TextBuffer.Unit.Tests.vcxproj"
text = project.read_text(encoding="utf-8")
anchor = '<ClCompile Include="ReflowTests.cpp" />'
if text.count(anchor) != 1:
    raise SystemExit("Native unit project anchor mismatch")
project.write_text(text.replace(anchor, anchor + '\n    <ClCompile Include="NativeOverlayTests.cpp" />'), encoding="utf-8")
print(f"Applied native feasibility tests to {expected}")
