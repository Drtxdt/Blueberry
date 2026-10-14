"""Extend only the independent Terminal prototype with validated menu frames."""
from pathlib import Path
import shutil
import subprocess
import sys

source = Path(sys.argv[1]).resolve()
assert subprocess.check_output(['git', '-C', str(source), 'rev-parse', 'HEAD'], text=True).strip() == '7ff11ee107c984e236960ec948b450ccbc10187b'
header = source / 'src/buffer/out/NativeOverlayFrame.hpp'
assert not header.exists()
shutil.copyfile(Path(__file__).with_name('NativeOverlayFrame.hpp'), header)
cpp = source / 'src/cascadia/TerminalCore/Terminal.cpp'
text = cpp.read_text(encoding='utf-8-sig')
text = text.replace('#include "Terminal.hpp"', '#include "Terminal.hpp"\n#include "../../buffer/out/NativeOverlayFrame.hpp"', 1)
anchor = '    if (command.size() > 96)'
assert text.count(anchor) == 1
text = text.replace(anchor, '    if (command.size() > 262240)', 1)
anchor = '    if (parts.size() == 2 && parts[0] == L"query")'
assert text.count(anchor) == 1
text = text.replace(anchor, '''    if (parts.size() == 6 && parts[0] == L"frame")
    {
        const auto generation = number(parts[1]);
        const auto top = number(parts[2]);
        const auto height = number(parts[3]);
        const auto width = number(parts[4]);
        if (generation && top && height && width && *height > 0 && *height <= 512 &&
            *width > 0 && *width <= 512 && *width < static_cast<uint64_t>(_mutableViewport.Width()) &&
            *top < static_cast<uint64_t>(_mutableViewport.Height()) &&
            *height <= static_cast<uint64_t>(_mutableViewport.Height()) - *top)
        {
            // Parse/render off-screen first: malformed frames preserve the old menu.
            auto frame = NativeOverlayFrame::Parse(parts[5], static_cast<til::CoordType>(*width),
                                                  static_cast<til::CoordType>(*height));
            const auto begin = _mutableViewport.Top() + static_cast<til::CoordType>(*top);
            if (frame && BeginNativeOverlayPrototype(begin, begin + static_cast<til::CoordType>(*height), *generation))
            {
                NativeOverlayFrame::Paint(*frame, *_mainBuffer, begin);
            }
        }
    }
    else if (parts.size() == 2 && parts[0] == L"capabilities")
    {
        if (const auto nonce = number(parts[1]))
        {
            ReturnResponse(L"\\x1b]7775;prototype;2;" + std::to_wstring(*nonce) + L";menu-sgr-rgb-utf16hex\\x07");
        }
    }
    else if (parts.size() == 2 && parts[0] == L"query")''', 1)
cpp.write_text(text, encoding='utf-8')
print('Applied bounded, staged Unicode/RGB menu frame interface')
