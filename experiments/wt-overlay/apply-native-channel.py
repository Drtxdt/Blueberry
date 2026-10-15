"""Isolated, owner-bound acknowledgement pipe; never synthesize editor input."""
from pathlib import Path
import shutil
import subprocess
import sys

source = Path(sys.argv[1]).resolve()
assert subprocess.check_output(['git', '-C', str(source), 'rev-parse', 'HEAD'], text=True).strip() == '7ff11ee107c984e236960ec948b450ccbc10187b'


def replace(relative, anchor, replacement):
    path = source / relative
    text = path.read_text(encoding='utf-8-sig')
    assert text.count(anchor) == 1, f'Native channel anchor mismatch: {relative}: {anchor[:70]}'
    path.write_text(text.replace(anchor, replacement, 1), encoding='utf-8')


header = source / 'src/cascadia/TerminalCore/NativeOverlayChannel.hpp'
assert not header.exists()
shutil.copyfile(Path(__file__).with_name(header.name), header)
terminal_h = 'src/cascadia/TerminalCore/Terminal.hpp'
terminal_cpp = 'src/cascadia/TerminalCore/Terminal.cpp'
replace(terminal_h, '#include "../../buffer/out/NativeOverlaySnapshot.hpp"',
        '#include "../../buffer/out/NativeOverlaySnapshot.hpp"\n#include "NativeOverlayChannel.hpp"')
replace(terminal_h, '    void NativeOverlayPrototype(std::wstring_view command) override;', '''    void NativeOverlayPrototype(std::wstring_view command) override;
    void SetNativeOverlayClosedCallback(std::function<void(std::wstring)> callback) { _nativeOverlayClosed = std::move(callback); }
    void RetireNativeOverlayChannel(std::wstring_view owner);''')
replace(terminal_h, '    uint64_t _nativeOverlayGeneration{};', '''    uint64_t _nativeOverlayGeneration{};
    std::unique_ptr<NativeOverlayChannel> _nativeOverlayChannel;
    std::function<void(std::wstring)> _nativeOverlayClosed;''')
replace(terminal_cpp, '    if (parts.size() == 6 && parts[0] == L"frame")', '''    bool privateFrame = false;
    if (parts.size() == 3 && parts[0] == L"channel")
    {
        if (const auto pid = number(parts[1]); pid && *pid <= MAXDWORD)
        {
            const std::wstring owner{ parts[2] };
            // A failed or foreign handshake must leave the old channel alone.
            auto channel = NativeOverlayChannel::Connect(owner, static_cast<DWORD>(*pid),
                [callback = _nativeOverlayClosed, owner]() { if (callback) { callback(owner); } });
            if (channel)
            {
                BeforeNativeOverlayMutation();
                _nativeOverlayChannel = std::move(channel);
                _nativeOverlayGeneration = 0;
            }
        }
        return;
    }
    if (parts.size() >= 2 && (parts[0] == L"native-frame" || parts[0] == L"native-close"))
    {
        if (!_nativeOverlayChannel || !_nativeOverlayChannel->Connected() ||
            parts[1] != _nativeOverlayChannel->Owner()) { return; }
        privateFrame = parts[0] == L"native-frame";
        parts[0] = privateFrame ? L"frame" : L"close";
        parts.erase(parts.begin() + 1);
    }
    else if (_nativeOverlayChannel && (parts[0] == L"frame" || parts[0] == L"close" || parts[0] == L"draw"))
    {
        return;
    }
    if (parts.size() == 6 && parts[0] == L"frame")''')
replace(terminal_cpp, '''            ReturnResponse(L"\\x1b]7775;frame;" + std::to_wstring(*generation) +
                           (presented ? L";presented\\x07" : L";rejected\\x07"));''', '''            if (privateFrame)
            {
                if (!_nativeOverlayChannel->Send("frame;" + std::to_string(*generation) +
                        (presented ? ";presented\\n" : ";rejected\\n")))
                {
                    BeforeNativeOverlayMutation();
                }
            }
            else
            {
                ReturnResponse(L"\\x1b]7775;frame;" + std::to_wstring(*generation) +
                               (presented ? L";presented\\x07" : L";rejected\\x07"));
            }''')
replace(terminal_cpp, '''    _nativeOverlayPrototype.reset();
    _mainBuffer->TriggerRedrawAll();''', '''    _nativeOverlayPrototype.reset();
    if (_nativeOverlayChannel)
    {
        _nativeOverlayChannel->Send("retired;" + std::to_string(generation) + "\\n");
    }
    _mainBuffer->TriggerRedrawAll();''')
path = source / terminal_cpp
path.write_text(path.read_text(encoding='utf-8') + '''
void Terminal::RetireNativeOverlayChannel(std::wstring_view owner)
{
    if (_nativeOverlayChannel && _nativeOverlayChannel->Owner() == owner)
    {
        BeforeNativeOverlayMutation();
        _nativeOverlayChannel.reset();
    }
}
''', encoding='utf-8')
replace('src/cascadia/TerminalControl/ControlCore.cpp', '''        _terminal->SetWriteInputCallback([this](std::wstring_view wstr) {
            _pendingResponses.append(wstr);
        });''', '''        _terminal->SetWriteInputCallback([this](std::wstring_view wstr) {
            _pendingResponses.append(wstr);
        });
        _terminal->SetNativeOverlayClosedCallback([weakThis = get_weak(), dispatcher = _dispatcher](std::wstring owner) {
            dispatcher.TryEnqueue(DispatcherQueuePriority::Normal, [weakThis, owner = std::move(owner)]() { try {
                if (const auto self = weakThis.get(); self && !self->_IsClosing())
                {
                    const auto lock = self->_terminal->LockForWriting();
                    self->_terminal->RetireNativeOverlayChannel(owner);
                }
            } CATCH_LOG() });
        });''')
print('Applied owner/PID-bound event-driven native acknowledgement channel')
