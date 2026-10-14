"""Expose the fixed-MENU proof through a private experimental OSC endpoint.

No installed Terminal, public protocol or arbitrary drawing payload is changed.
The existing Terminal parser owns chunking; ordinary parsed actions retire the
overlay, whereas stale private commands cannot retire a newer generation.
"""
from pathlib import Path
import re
import subprocess
import sys

source = Path(sys.argv[1]).resolve()
assert subprocess.check_output(['git', '-C', str(source), 'rev-parse', 'HEAD'], text=True).strip() == '7ff11ee107c984e236960ec948b450ccbc10187b'

def replace(relative, anchor, replacement):
    p = source / relative
    s = p.read_text(encoding='utf-8-sig')
    assert s.count(anchor) == 1, f'VT interface anchor mismatch: {relative}'
    p.write_text(s.replace(anchor, replacement), encoding='utf-8')

for header in ('src/terminal/adapter/ITermDispatch.hpp', 'src/terminal/adapter/ITerminalApi.hpp'):
    p = source / header
    s = p.read_text(encoding='utf-8-sig')
    anchor = '    virtual void Print(const wchar_t wchPrintable) = 0;' if header.endswith('ITermDispatch.hpp') else '        virtual ~ITerminalApi() = default;'
    indentation = '    ' if header.endswith('ITermDispatch.hpp') else '        '
    extension = '\n' + indentation + 'virtual void BeforeNativeOverlayMutation() {}\n' + indentation + 'virtual void NativeOverlayPrototype(std::wstring_view) {}'
    replace(header, anchor, anchor + extension)

replace('src/terminal/adapter/adaptDispatch.hpp', '        void PrintString(const std::wstring_view string) override;', '''        void PrintString(const std::wstring_view string) override;
        void BeforeNativeOverlayMutation() override;
        void NativeOverlayPrototype(std::wstring_view command) override;''')
p = source / 'src/terminal/adapter/adaptDispatch.cpp'
p.write_text(p.read_text(encoding='utf-8-sig') + '''
void AdaptDispatch::BeforeNativeOverlayMutation()
{
    _api.BeforeNativeOverlayMutation();
}

void AdaptDispatch::NativeOverlayPrototype(std::wstring_view command)
{
    _api.NativeOverlayPrototype(command);
}
''', encoding='utf-8')

replace('src/cascadia/TerminalCore/Terminal.hpp', '    bool EndNativeOverlayPrototype(uint64_t generation);', '''    bool EndNativeOverlayPrototype(uint64_t generation);
    void BeforeNativeOverlayMutation() override;
    void NativeOverlayPrototype(std::wstring_view command) override;''')
replace('src/cascadia/TerminalCore/Terminal.cpp', '''void Terminal::Write(std::wstring_view stringView)
{
    // External output can touch any row or scroll. Retire the overlay first.
    if (_nativeOverlayPrototype && !EndNativeOverlayPrototype(_nativeOverlayGeneration))
    {
        THROW_HR(E_FAIL);
    }
    _stateMachine->ProcessString(stringView);
}''', r'''void Terminal::BeforeNativeOverlayMutation()
{
    if (_nativeOverlayPrototype && !EndNativeOverlayPrototype(_nativeOverlayGeneration))
    {
        THROW_HR(E_FAIL);
    }
}

void Terminal::NativeOverlayPrototype(std::wstring_view command)
{
    // Bounded proof endpoint: draw;generation;firstRow;exclusiveLastRow,
    // or close;generation. Rows are absolute native buffer offsets.
    if (command.size() > 96)
    {
        return;
    }
    std::vector<std::wstring_view> parts;
    while (true)
    {
        const auto separator = command.find(L';');
        parts.push_back(command.substr(0, separator));
        if (separator == std::wstring_view::npos)
        {
            break;
        }
        command.remove_prefix(separator + 1);
    }
    const auto number = [](std::wstring_view text) -> std::optional<uint64_t> {
        if (text.empty()) { return std::nullopt; }
        uint64_t value = 0;
        for (const auto ch : text)
        {
            if (ch < L'0' || ch > L'9') { return std::nullopt; }
            const auto digit = static_cast<uint64_t>(ch - L'0');
            if (value > (UINT64_MAX - digit) / 10) { return std::nullopt; }
            value = value * 10 + digit;
        }
        return value;
    };
    if (parts.size() == 2 && parts[0] == L"query")
    {
        if (const auto nonce = number(parts[1]))
        {
            // Advertise the bounded proof only, never production support.
            ReturnResponse(L"\x1b]7775;prototype;1;" + std::to_wstring(*nonce) + L";fixed-menu\x07");
        }
    }
    else if (parts.size() == 2 && parts[0] == L"close")
    {
        if (const auto generation = number(parts[1]))
        {
            EndNativeOverlayPrototype(*generation);
        }
    }
    else if (parts.size() == 4 && parts[0] == L"draw")
    {
        const auto generation = number(parts[1]);
        const auto begin = number(parts[2]);
        const auto end = number(parts[3]);
        if (generation && begin && end && *begin < *end &&
            *end <= static_cast<uint64_t>(_mainBuffer->GetSize().Height()) &&
            BeginNativeOverlayPrototype(static_cast<til::CoordType>(*begin), static_cast<til::CoordType>(*end), *generation))
        {
            PaintNativeOverlayPrototype(*generation);
        }
    }
}

void Terminal::Write(std::wstring_view stringView)
{
    _stateMachine->ProcessString(stringView);
}''')

engine = source / 'src/terminal/parser/OutputStateMachineEngine.cpp'
text = engine.read_text(encoding='utf-8-sig')
for name in ('ActionExecute', 'ActionPrint', 'ActionPrintString', 'ActionEscDispatch', 'ActionVt52EscDispatch', 'ActionCsiDispatch', 'ActionDcsDispatch'):
    pattern = rf'(OutputStateMachineEngine::{name}\([^\n]*\)\n\{{)'
    text, count = re.subn(pattern, r'\1\n    _dispatch->BeforeNativeOverlayMutation();', text)
    assert count == 1, f'Ordinary VT action anchor mismatch: {name}'
anchor = '''bool OutputStateMachineEngine::ActionOscDispatch(const size_t parameter, const std::wstring_view string)
{'''
assert text.count(anchor) == 1
text = text.replace(anchor, anchor + '''
    // Private independent prototype only; the standard parser owns chunks.
    if (parameter == 7775)
    {
        _dispatch->NativeOverlayPrototype(string);
        return true;
    }
    _dispatch->BeforeNativeOverlayMutation();''')
engine.write_text(text, encoding='utf-8')
print('Applied parser-owned experimental OSC 7775 native overlay proof')
