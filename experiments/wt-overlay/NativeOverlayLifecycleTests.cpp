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

private:
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
