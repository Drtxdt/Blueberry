"""Bounded native Terminal lifecycle hook; no parser/GUI/installation changes."""
from pathlib import Path
import shutil
import subprocess
import sys

source = Path(sys.argv[1]).resolve()
assert subprocess.check_output(['git', '-C', str(source), 'rev-parse', 'HEAD'], text=True).strip() == '7ff11ee107c984e236960ec948b450ccbc10187b'

def replace(relative, anchor, replacement):
    path = source / relative
    text = path.read_text(encoding='utf-8-sig')
    if text.count(anchor) != 1:
        raise SystemExit(f'Lifecycle anchor mismatch: {relative}')
    path.write_text(text.replace(anchor, replacement), encoding='utf-8')

header = 'src/cascadia/TerminalCore/Terminal.hpp'
replace(header, '#include "../../buffer/out/textBuffer.hpp"', '#include "../../buffer/out/textBuffer.hpp"\n#include "../../buffer/out/NativeOverlaySnapshot.hpp"')
replace(header, '    TextBuffer& GetTextBuffer() const noexcept override;', '''    TextBuffer& GetTextBuffer() const noexcept override;
    // Independent native API prototype. Caller holds the Terminal write lock.
    bool BeginNativeOverlayPrototype(til::CoordType begin, til::CoordType end, uint64_t generation);
    bool PaintNativeOverlayPrototype(uint64_t generation);
    bool EndNativeOverlayPrototype(uint64_t generation);''')
replace(header, '    std::unique_ptr<TextBuffer> _mainBuffer;', '''    std::unique_ptr<TextBuffer> _mainBuffer;
    std::unique_ptr<NativeOverlaySnapshot> _nativeOverlayPrototype;
    uint64_t _nativeOverlayGeneration{};
    til::CoordType _nativeOverlayBegin{};
    til::CoordType _nativeOverlayEnd{};''')
cpp = 'src/cascadia/TerminalCore/Terminal.cpp'
replace(cpp, '''[[nodiscard]] HRESULT Terminal::UserResize(const til::size requestedSize) noexcept
try
{''', '''[[nodiscard]] HRESULT Terminal::UserResize(const til::size requestedSize) noexcept
try
{
    // Restore while the old buffer dimensions and hyperlink IDs are intact.
    if (_nativeOverlayPrototype && !EndNativeOverlayPrototype(_nativeOverlayGeneration))
    {
        return E_FAIL;
    }''')
replace(cpp, '''void Terminal::Write(std::wstring_view stringView)
{
    _stateMachine->ProcessString(stringView);
}''', '''bool Terminal::BeginNativeOverlayPrototype(til::CoordType begin, til::CoordType end, uint64_t generation)
{
    if (generation <= _nativeOverlayGeneration || _inAltBuffer() ||
        (_nativeOverlayPrototype && !EndNativeOverlayPrototype(_nativeOverlayGeneration)))
    {
        return false;
    }
    _nativeOverlayPrototype = std::make_unique<NativeOverlaySnapshot>(*_mainBuffer, begin, end, generation);
    _nativeOverlayBegin = begin;
    _nativeOverlayEnd = end;
    _nativeOverlayGeneration = generation;
    return true;
}

bool Terminal::PaintNativeOverlayPrototype(uint64_t generation)
{
    if (!_nativeOverlayPrototype || generation != _nativeOverlayGeneration ||
        _mainBuffer->GetSize().Width() < 4)
    {
        return false;
    }
    for (auto y = _nativeOverlayBegin; y < _nativeOverlayEnd; ++y)
    {
        auto& row = _mainBuffer->GetMutableRowByOffset(y);
        row.Reset(TextAttribute{ 0x4f });
        row.ReplaceCharacters(0, 4, L"MENU");
    }
    _mainBuffer->TriggerRedrawAll();
    return true;
}

bool Terminal::EndNativeOverlayPrototype(uint64_t generation)
{
    if (!_nativeOverlayPrototype || generation != _nativeOverlayGeneration ||
        !_nativeOverlayPrototype->RestoreBeforeResize(*_mainBuffer, generation))
    {
        return false;
    }
    _nativeOverlayPrototype.reset();
    _mainBuffer->TriggerRedrawAll();
    return true;
}

void Terminal::Write(std::wstring_view stringView)
{
    // External output can touch any row or scroll. Retire the overlay first.
    if (_nativeOverlayPrototype && !EndNativeOverlayPrototype(_nativeOverlayGeneration))
    {
        THROW_HR(E_FAIL);
    }
    _stateMachine->ProcessString(stringView);
}''')
tests = source / 'src/cascadia/UnitTests_TerminalCore/NativeOverlayLifecycleTests.cpp'
assert not tests.exists()
shutil.copyfile(Path(__file__).with_name('NativeOverlayLifecycleTests.cpp'), tests)
replace('src/cascadia/UnitTests_TerminalCore/UnitTests.vcxproj', '<ClCompile Include="TerminalBufferTests.cpp" />', '<ClCompile Include="TerminalBufferTests.cpp" />\n    <ClCompile Include="NativeOverlayLifecycleTests.cpp" />')
print('Applied bounded Terminal::UserResize / Terminal::Write lifecycle hook')
