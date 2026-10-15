// Copyright (c) Blueberry contributors. Licensed under the MIT license.
#include "pch.h"
#include "../renderer/inc/DummyRenderer.hpp"
#include "../cascadia/TerminalCore/Terminal.hpp"

using namespace WEX::TestExecution;
using Microsoft::Terminal::Core::Terminal;

class NativeOverlayLifecycleTests
{
    TEST_CLASS(NativeOverlayLifecycleTests);

    TEST_METHOD(ActualUserResizeRestoresBeforeReflowAndRejectsOldGeneration)
    {
        Terminal reference{ Terminal::TestDummyMarker{} };
        Terminal overlaid{ Terminal::TestDummyMarker{} };
        DummyRenderer referenceRenderer{ &reference };
        DummyRenderer overlayRenderer{ &overlaid };
        reference.Create({ 40, 12 }, 100, referenceRenderer);
        overlaid.Create({ 40, 12 }, 100, overlayRenderer);
        const auto history = L"\x1b[38;2;17;123;239m\x1b[58;2;9;87;165m\x1b[4:3m\x1b]8;id=original;https://example.invalid/history\x1b\\\u4e2d\U0001f680e\u0301-abcdefghijklmnopqrstuvwxyz-0123456789\x1b]8;;\x1b\\\x1b[0m\r\n$ ";
        reference.Write(history);
        overlaid.Write(history);
        uint64_t generation = 17;
        for (const auto width : { 20, 50, 13, 40 })
        {
            VERIFY_IS_TRUE(overlaid.BeginNativeOverlayPrototype(0, 2, generation));
            VERIFY_IS_TRUE(overlaid.PaintNativeOverlayPrototype(generation));
            VERIFY_IS_FALSE(overlaid.PaintNativeOverlayPrototype(generation - 1));
            VERIFY_IS_FALSE(overlaid.EndNativeOverlayPrototype(generation - 1));
            VERIFY_SUCCEEDED(reference.UserResize({ width, 12 }));
            VERIFY_SUCCEEDED(overlaid.UserResize({ width, 12 }));
            VERIFY_IS_FALSE(overlaid.PaintNativeOverlayPrototype(generation));
            VERIFY_IS_FALSE(overlaid.BeginNativeOverlayPrototype(0, 2, generation));
            Compare(reference.GetTextBuffer(), overlaid.GetTextBuffer());
            ++generation;
        }
    }

    TEST_METHOD(OrdinaryOutputRestoresBeforeWritingCoveredHistory)
    {
        Terminal reference{ Terminal::TestDummyMarker{} };
        Terminal overlaid{ Terminal::TestDummyMarker{} };
        DummyRenderer referenceRenderer{ &reference };
        DummyRenderer overlayRenderer{ &overlaid };
        reference.Create({ 40, 12 }, 100, referenceRenderer);
        overlaid.Create({ 40, 12 }, 100, overlayRenderer);
        reference.Write(L"old history\r\n$ ");
        overlaid.Write(L"old history\r\n$ ");
        VERIFY_IS_TRUE(overlaid.BeginNativeOverlayPrototype(0, 1, 18));
        VERIFY_IS_TRUE(overlaid.PaintNativeOverlayPrototype(18));
        // Ordinary output deliberately writes in the covered row and allocates
        // a new hyperlink. Closing must not put the old row back over it.
        const auto output = L"\x1b[1;1H\x1b]8;id=new;https://example.invalid/new\x1b\\NEW\x1b]8;;\x1b\\";
        reference.Write(output);
        overlaid.Write(output);
        VERIFY_IS_FALSE(overlaid.EndNativeOverlayPrototype(18));
        Compare(reference.GetTextBuffer(), overlaid.GetTextBuffer());
    }

    TEST_METHOD(FragmentedPrivateDrawAndStaleClosePreserveCurrentOverlay)
    {
        Terminal reference{ Terminal::TestDummyMarker{} };
        Terminal overlaid{ Terminal::TestDummyMarker{} };
        DummyRenderer referenceRenderer{ &reference };
        DummyRenderer overlayRenderer{ &overlaid };
        reference.Create({ 40, 12 }, 100, referenceRenderer);
        overlaid.Create({ 40, 12 }, 100, overlayRenderer);
        const auto history = L"\x1b[38;2;17;123;239mhistory\x1b[0m\r\n$ ";
        reference.Write(history);
        overlaid.Write(history);
        const auto sendChunks = [&](std::wstring_view packet) {
            for (const auto& ch : packet) { overlaid.Write({ &ch, 1 }); }
        };
        sendChunks(L"\x1b]7775;draw;31;0;1\x07");
        VERIFY_IS_TRUE(overlaid.GetTextBuffer().GetRowByOffset(0).GetText().substr(0, 4) == L"MENU");
        sendChunks(L"\x1b]7775;draw;32;0;1\x1b\\");
        sendChunks(L"\x1b]7775;close;31\x07");
        sendChunks(L"\x1b]7775;draw;18446744073709551616;0;1\x07");
        VERIFY_IS_TRUE(overlaid.GetTextBuffer().GetRowByOffset(0).GetText().substr(0, 4) == L"MENU");
        sendChunks(L"\x1b]7775;close;32\x07");
        Compare(reference.GetTextBuffer(), overlaid.GetTextBuffer());
    }

    TEST_METHOD(PrivateOverlayRetiresBeforeScrollingAndAlternateScreen)
    {
        Terminal reference{ Terminal::TestDummyMarker{} };
        Terminal overlaid{ Terminal::TestDummyMarker{} };
        DummyRenderer referenceRenderer{ &reference };
        DummyRenderer overlayRenderer{ &overlaid };
        reference.Create({ 40, 12 }, 100, referenceRenderer);
        overlaid.Create({ 40, 12 }, 100, overlayRenderer);
        reference.Write(L"history\r\n$ ");
        overlaid.Write(L"history\r\n$ ");
        overlaid.Write(L"\x1b]7775;draw;33;0;1\x07");
        const auto alternate = L"\x1b[?1049hALT-\u4e2d\U0001f680\x1b[?1049l";
        reference.Write(alternate);
        overlaid.Write(alternate);
        VERIFY_IS_FALSE(overlaid.EndNativeOverlayPrototype(33));
        Compare(reference.GetTextBuffer(), overlaid.GetTextBuffer());
        overlaid.Write(L"\x1b]7775;draw;34;0;1\x07");
        const auto scroll = L"\x1b[12;1H\r\nNEW OUTPUT";
        reference.Write(scroll);
        overlaid.Write(scroll);
        VERIFY_IS_FALSE(overlaid.EndNativeOverlayPrototype(34));
        Compare(reference.GetTextBuffer(), overlaid.GetTextBuffer());
    }

    TEST_METHOD(PrototypeCapabilityReplyPreservesActiveOverlay)
    {
        Terminal term{ Terminal::TestDummyMarker{} };
        DummyRenderer renderer{ &term };
        term.Create({ 40, 12 }, 100, renderer);
        std::wstring response;
        term.SetWriteInputCallback([&](std::wstring_view text) { response.assign(text); });
        term.Write(L"history\r\n$ ");
        term.Write(L"\x1b]7775;draw;35;0;1\x07");
        const std::wstring_view query = L"\x1b]7775;query;123\x07";
        for (const auto& ch : query) { term.Write({ &ch, 1 }); }
        VERIFY_IS_TRUE(response == L"\x1b]7775;prototype;1;123;fixed-menu\x07");
        VERIFY_IS_TRUE(term.GetTextBuffer().GetRowByOffset(0).GetText().substr(0, 4) == L"MENU");
        term.Write(L"\x1b]7775;close;35\x07");
        VERIFY_IS_TRUE(term.GetTextBuffer().GetRowByOffset(0).GetText().substr(0, 7) == L"history");
    }

    TEST_METHOD(PrivateCoordinatesFollowViewportAfterHistoryScroll)
    {
        Terminal reference{ Terminal::TestDummyMarker{} };
        Terminal overlaid{ Terminal::TestDummyMarker{} };
        DummyRenderer referenceRenderer{ &reference };
        DummyRenderer overlayRenderer{ &overlaid };
        reference.Create({ 40, 12 }, 100, referenceRenderer);
        overlaid.Create({ 40, 12 }, 100, overlayRenderer);
        for (int i = 0; i < 30; ++i)
        {
            reference.Write(L"history line\r\n");
            overlaid.Write(L"history line\r\n");
        }
        const auto top = overlaid.GetViewport().Top();
        VERIFY_IS_TRUE(top > 0);
        overlaid.Write(L"\x1b]7775;draw;36;0;1\x07");
        VERIFY_IS_TRUE(overlaid.GetTextBuffer().GetRowByOffset(top).GetText().substr(0, 4) == L"MENU");
        VERIFY_IS_TRUE(overlaid.GetTextBuffer().GetRowByOffset(0).GetText() == reference.GetTextBuffer().GetRowByOffset(0).GetText());
        overlaid.Write(L"\x1b]7775;close;36\x07");
        Compare(reference.GetTextBuffer(), overlaid.GetTextBuffer());
    }

    TEST_METHOD(ValidatedFramePreservesUnicodeRgbAndRestoresBeforeResize)
    {
        Terminal reference{ Terminal::TestDummyMarker{} };
        Terminal overlaid{ Terminal::TestDummyMarker{} };
        DummyRenderer referenceRenderer{ &reference };
        DummyRenderer overlayRenderer{ &overlaid };
        reference.Create({ 40, 12 }, 100, referenceRenderer);
        overlaid.Create({ 40, 12 }, 100, overlayRenderer);
        const auto history = L"\x1b[58;2;8;9;10m\x1b[4:3m\x1b]8;id=kept;https://example.invalid/original\x1b\\original history-abcdefghijklmnopqrstuvwxyz\x1b]8;;\x1b\\\x1b[0m\r\n$ ";
        reference.Write(history);
        overlaid.Write(history);
        const auto command = Frame(50, 0, 2, 20, L"\x1b[38;2;17;123;239;48;2;23;45;67;1m\u4e2d\U0001f680e\u0301 menu\x1b[0m\nborder");
        overlaid.Write(command);
        const auto& row = overlaid.GetTextBuffer().GetRowByOffset(0);
        VERIFY_IS_TRUE(row.GlyphAt(0) == L"\u4e2d");
        VERIFY_IS_TRUE(row.GlyphAt(2) == L"\U0001f680");
        VERIFY_IS_TRUE(row.GetAttrByColumn(0).GetForeground() == TextColor{ RGB(17, 123, 239) });
        VERIFY_IS_TRUE(row.GetAttrByColumn(0).GetBackground() == TextColor{ RGB(23, 45, 67) });
        VERIFY_IS_TRUE(row.GetAttrByColumn(0).IsIntense());
        VERIFY_IS_TRUE(row.GlyphAt(25) == reference.GetTextBuffer().GetRowByOffset(0).GlyphAt(25));
        VERIFY_IS_TRUE(overlaid.GetTextBuffer().GetCursor().GetPosition() == reference.GetTextBuffer().GetCursor().GetPosition());
        VERIFY_IS_TRUE(overlaid.GetTextBuffer().GetCurrentAttributes() == reference.GetTextBuffer().GetCurrentAttributes());
        VERIFY_SUCCEEDED(reference.UserResize({ 20, 12 }));
        VERIFY_SUCCEEDED(overlaid.UserResize({ 20, 12 }));
        Compare(reference.GetTextBuffer(), overlaid.GetTextBuffer());
    }

    TEST_METHOD(MalformedFrameCannotEraseCurrentMenuOrExecuteVt)
    {
        Terminal term{ Terminal::TestDummyMarker{} };
        DummyRenderer renderer{ &term };
        term.Create({ 40, 12 }, 100, renderer);
        term.Write(L"original history\r\n$ ");
        std::wstring reply;
        term.SetWriteInputCallback([&](std::wstring_view text) { reply.assign(text); });
        term.Write(L"\x1b]7775;draw;51;0;1\x07");
        for (const auto& command : {
                 Frame(52, 0, 1, 20, L"\x1b[2J"),
                 Frame(52, 0, 1, 20, L"\x1b]52;c;payload\x07"),
                 Frame(52, 0, 1, 20, L"bad\rtext"),
                 Frame(52, 0, 2, 20, L"wrong row count"),
                 Frame(52, 0, 1, 2, L"too wide"),
                 std::wstring{ L"\x1b]7775;frame;52;0;1;20;d800\x07" },
                 std::wstring{ L"\x1b]7775;frame;52;0;1;20;zzzz\x07" } })
        {
            term.Write(command);
            VERIFY_IS_TRUE(reply == L"\x1b]7775;frame;52;rejected\x07");
            VERIFY_IS_TRUE(term.GetTextBuffer().GetRowByOffset(0).GetText().substr(0, 4) == L"MENU");
        }
        term.Write(Frame(52, 0, 1, 20, L"GOOD"));
        VERIFY_IS_TRUE(reply == L"\x1b]7775;frame;52;presented\x07");
        term.Write(Frame(51, 0, 1, 20, L"OLD"));
        VERIFY_IS_TRUE(reply == L"\x1b]7775;frame;51;rejected\x07");
        term.Write(L"\x1b]7775;close;51\x07");
        VERIFY_IS_TRUE(term.GetTextBuffer().GetRowByOffset(0).GetText().substr(0, 4) == L"GOOD");
        term.Write(L"\x1b]7775;close;52\x07");
        VERIFY_IS_TRUE(term.GetTextBuffer().GetRowByOffset(0).GetText().substr(0, 16) == L"original history");
    }

    TEST_METHOD(FragmentedFrameCapabilitiesAndOrdinaryOutput)
    {
        Terminal reference{ Terminal::TestDummyMarker{} };
        Terminal overlaid{ Terminal::TestDummyMarker{} };
        DummyRenderer referenceRenderer{ &reference };
        DummyRenderer overlayRenderer{ &overlaid };
        reference.Create({ 40, 12 }, 100, referenceRenderer);
        overlaid.Create({ 40, 12 }, 100, overlayRenderer);
        reference.Write(L"history\r\n$ ");
        overlaid.Write(L"history\r\n$ ");
        const auto command = Frame(53, 0, 1, 20, L"\x1b[38;2;1;2;3mmenu\x1b[0m");
        for (const auto& ch : command) { overlaid.Write({ &ch, 1 }); }
        std::wstring response;
        overlaid.SetWriteInputCallback([&](std::wstring_view text) { response.assign(text); });
        overlaid.Write(L"\x1b]7775;capabilities;987\x07");
        VERIFY_IS_TRUE(response == L"\x1b]7775;prototype;2;987;menu-sgr-rgb-utf16hex\x07");
        VERIFY_IS_TRUE(overlaid.GetTextBuffer().GetRowByOffset(0).GetText().substr(0, 4) == L"menu");
        const auto output = L"\x1b[1;1H\x1b]8;id=new;https://example.invalid/new\x1b\\NEW\x1b]8;;\x1b\\";
        reference.Write(output);
        overlaid.Write(output);
        overlaid.Write(L"\x1b]7775;close;53\x07");
        Compare(reference.GetTextBuffer(), overlaid.GetTextBuffer());
    }

    TEST_METHOD(FrameWithoutSgrInheritsTheExistingVtAttributes)
    {
        Terminal term{ Terminal::TestDummyMarker{} };
        DummyRenderer renderer{ &term };
        term.Create({ 40, 12 }, 100, renderer);
        term.Write(L"history\r\n$ \x1b[38;2;12;34;56m\x1b]8;id=active;https://example.invalid/active\x1b\\");
        const auto before = term.GetTextBuffer().GetCurrentAttributes();
        term.Write(Frame(55, 0, 1, 20, L"plain\x1b[0m menu"));
        VERIFY_IS_TRUE(term.GetTextBuffer().GetRowByOffset(0).GetAttrByColumn(0) == before);
        VERIFY_IS_TRUE(term.GetTextBuffer().GetCurrentAttributes() == before);
        VERIFY_IS_TRUE(term.GetTextBuffer().GetRowByOffset(0).GetAttrByColumn(6).GetHyperlinkId() == before.GetHyperlinkId());
        VERIFY_IS_TRUE(term.GetTextBuffer().GetRowByOffset(0).GetAttrByColumn(6).GetForeground() == TextAttribute{}.GetForeground());
        term.Write(L"\x1b]7775;close;55\x07");
        VERIFY_IS_TRUE(term.GetTextBuffer().GetRowByOffset(0).GetText().substr(0, 7) == L"history");
    }

    TEST_METHOD(ConnectionResetRestoresBeforeResettingNativeState)
    {
        Terminal reference{ Terminal::TestDummyMarker{} };
        Terminal overlaid{ Terminal::TestDummyMarker{} };
        DummyRenderer referenceRenderer{ &reference };
        DummyRenderer overlayRenderer{ &overlaid };
        reference.Create({ 40, 12 }, 100, referenceRenderer);
        overlaid.Create({ 40, 12 }, 100, overlayRenderer);
        reference.Write(L"\x1b[38;2;12;34;56mhistory\x1b[0m\r\n$ ");
        overlaid.Write(L"\x1b[38;2;12;34;56mhistory\x1b[0m\r\n$ ");
        overlaid.Write(Frame(54, 0, 1, 20, L"menu"));
        reference.HardResetWithoutErase();
        overlaid.HardResetWithoutErase();
        Compare(reference.GetTextBuffer(), overlaid.GetTextBuffer());
    }

    TEST_METHOD(PrivatePipeAcknowledgesFramesAndRetiresOnDisconnect)
    {
        const auto owner = L"18fbcb598bcd41e4934c892bf39d8536";
        const auto name = std::wstring{ L"\\\\.\\pipe\\Blueberry.TerminalOverlay." } + owner;
        wil::unique_handle server{ CreateNamedPipeW(name.c_str(), PIPE_ACCESS_DUPLEX | FILE_FLAG_OVERLAPPED | FILE_FLAG_FIRST_PIPE_INSTANCE,
            PIPE_TYPE_BYTE | PIPE_READMODE_BYTE | PIPE_WAIT | PIPE_REJECT_REMOTE_CLIENTS, 1, 4096, 4096, 0, nullptr) };
        VERIFY_IS_TRUE(server.get() != INVALID_HANDLE_VALUE);
        wil::unique_handle connected{ CreateEventW(nullptr, TRUE, FALSE, nullptr) };
        OVERLAPPED connection{};
        connection.hEvent = connected.get();
        VERIFY_IS_TRUE(!ConnectNamedPipe(server.get(), &connection) && GetLastError() == ERROR_IO_PENDING);
        std::atomic<bool> closed{ false };
        std::wstring input;
        Terminal term{ Terminal::TestDummyMarker{} };
        DummyRenderer renderer{ &term };
        term.Create({ 40, 12 }, 100, renderer);
        term.Write(L"history\r\n$ ");
        term.SetWriteInputCallback([&](std::wstring_view text) { input.append(text); });
        term.SetNativeOverlayClosedCallback([&](std::wstring value) { if (value == owner) { closed.store(true); } });
        term.Write(L"\x1b]7775;channel;" + std::to_wstring(GetCurrentProcessId()) + L";" + owner + L"\x07");
        VERIFY_IS_TRUE(WaitForSingleObject(connected.get(), 2000) == WAIT_OBJECT_0);
        VERIFY_IS_TRUE(ReadPipe(server.get()) == "ready;3;18fbcb598bcd41e4934c892bf39d8536\n");
        auto frame = Frame(61, 0, 1, 20, L"menu");
        frame.replace(frame.find(L"frame;"), 6, std::wstring{ L"native-frame;" } + owner + L";");
        frame.replace(frame.find(L";0;1;20;"), 8, L";0;3;1;20;");
        term.Write(frame);
        VERIFY_IS_TRUE(ReadPipe(server.get()) == "frame;61;presented\n");
        VERIFY_IS_TRUE(input.empty());
        term.Write(L"\x1b]7775;native-close;00000000000000000000000000000000;61\x07");
        VERIFY_IS_TRUE(term.GetTextBuffer().GetRowByOffset(0).GetText().substr(3, 4) == L"menu");
        server.reset();
        for (int attempt = 0; attempt < 200 && !closed.load(); ++attempt) { Sleep(10); }
        VERIFY_IS_TRUE(closed.load());
        term.RetireNativeOverlayChannel(owner);
        VERIFY_IS_TRUE(term.GetTextBuffer().GetRowByOffset(0).GetText().substr(0, 7) == L"history");
        VERIFY_IS_TRUE(input.empty());
    }

private:
    static std::string ReadPipe(HANDLE pipe)
    {
        wil::unique_handle event{ CreateEventW(nullptr, TRUE, FALSE, nullptr) };
        OVERLAPPED operation{};
        operation.hEvent = event.get();
        char data[192]{};
        DWORD count{};
        if (!ReadFile(pipe, data, sizeof(data), &count, &operation))
        {
            VERIFY_IS_TRUE(GetLastError() == ERROR_IO_PENDING);
            const auto wait = WaitForSingleObject(event.get(), 2000);
            if (wait != WAIT_OBJECT_0) { CancelIoEx(pipe, &operation); }
            const auto completed = GetOverlappedResult(pipe, &operation, &count, TRUE);
            VERIFY_IS_TRUE(wait == WAIT_OBJECT_0 && completed);
        }
        return { data, count };
    }

    static std::wstring Frame(uint64_t generation, unsigned top, unsigned height, unsigned width, std::wstring_view text)
    {
        static constexpr wchar_t digits[] = L"0123456789abcdef";
        std::wstring encoded;
        for (const auto ch : text)
        {
            for (int shift = 12; shift >= 0; shift -= 4) { encoded.push_back(digits[(ch >> shift) & 15]); }
        }
        return L"\x1b]7775;frame;" + std::to_wstring(generation) + L";" + std::to_wstring(top) +
               L";" + std::to_wstring(height) + L";" + std::to_wstring(width) + L";" + encoded + L"\x07";
    }

    static void Compare(const TextBuffer& a, const TextBuffer& b)
    {
        SetVerifyOutput output{ VerifyOutputSettings::LogOnlyFailures };
        VERIFY_IS_TRUE(a.GetSize().Dimensions() == b.GetSize().Dimensions());
        VERIFY_IS_TRUE(a.GetCursor().GetPosition() == b.GetCursor().GetPosition());
        VERIFY_IS_TRUE(a.GetCurrentAttributes() == b.GetCurrentAttributes());
        const auto size = a.GetSize().Dimensions();
        for (int y = 0; y < size.height; ++y)
        {
            const auto& x = a.GetRowByOffset(y);
            const auto& z = b.GetRowByOffset(y);
            VERIFY_IS_TRUE(x.WasWrapForced() == z.WasWrapForced());
            VERIFY_IS_TRUE(x.WasDoubleBytePadded() == z.WasDoubleBytePadded());
            VERIFY_IS_TRUE(x.GetLineRendition() == z.GetLineRendition());
            for (int col = 0; col < size.width; ++col)
            {
                VERIFY_IS_TRUE(x.GlyphAt(col) == z.GlyphAt(col));
                VERIFY_IS_TRUE(x.DbcsAttrAt(col) == z.DbcsAttrAt(col));
                const auto attr = x.GetAttrByColumn(col);
                VERIFY_IS_TRUE(attr == z.GetAttrByColumn(col));
                if (attr.IsHyperlink())
                {
                    VERIFY_IS_TRUE(a.GetHyperlinkUriFromId(attr.GetHyperlinkId()) == b.GetHyperlinkUriFromId(attr.GetHyperlinkId()));
                    VERIFY_IS_TRUE(a.GetCustomIdFromId(attr.GetHyperlinkId()) == b.GetCustomIdFromId(attr.GetHyperlinkId()));
                }
            }
        }
    }
};
