// Copyright (c) Blueberry contributors. Licensed under the MIT license.
#include "precomp.h"
#include "WexTestClass.h"
#include "../../inc/consoletaeftemplates.hpp"
#include "../NativeOverlaySnapshot.hpp"
#include "../../renderer/inc/DummyRenderer.hpp"

using namespace WEX::TestExecution;

class NativeOverlayTests
{
    TEST_CLASS(NativeOverlayTests);
    inline static DummyRenderer renderer{};

    static std::unique_ptr<TextBuffer> Fixture()
    {
        auto buffer = std::make_unique<TextBuffer>(til::size{ 24, 64 }, TextAttribute{ 7 }, 0, false, &renderer);
        const auto link = buffer->GetHyperlinkId(L"https://example.invalid/history?x=1#anchor", L"original");
        buffer->AddHyperlinkToMap(L"https://example.invalid/history?x=1#anchor", link);
        for (int y = 0; y < 3; ++y)
        {
            auto& row = buffer->GetMutableRowByOffset(y);
            for (int x = 0; x < 24; ++x)
            {
                TextAttribute attr{ RGB(17 + x, 123, 239), RGB(201, 32 + y, 63), RGB(9, 87, 165) };
                attr.SetIntense(x % 2 == 0);
                attr.SetFaint(x % 3 == 0);
                attr.SetItalic(true);
                attr.SetBlinking(x % 2 != 0);
                attr.SetInvisible(x % 4 == 0);
                attr.SetCrossedOut(true);
                attr.SetUnderlineStyle(static_cast<UnderlineStyle>(x % 6));
                attr.SetOverlined(true);
                attr.SetReverseVideo(x % 2 == 0);
                attr.SetProtected(true);
                attr.SetHyperlinkId(link);
                attr.SetMarkAttributes(MarkKind::Command);
                if (x == 1) { attr.SetDefaultForeground(); }
                if (x == 2) { attr.SetIndexedForeground256(129); }
                if (x == 3) { attr.SetDefaultUnderlineColor(); }
                if (x == 4) { attr.SetUnderlineColor(TextColor{ static_cast<BYTE>(119), true }); }
                row.ReplaceCharacters(x, 1, L"w");
                row.ReplaceAttributes(x, x + 1, attr);
            }
            row.ReplaceCharacters(5, 2, L"\u4e2d");
            row.ReplaceCharacters(8, 2, L"\U0001f680");
            row.ReplaceCharacters(11, 1, L"e\u0301");
            row.SetWrapForced(y < 2);
        }
        buffer->GetMutableRowByOffset(0).StartPrompt();
        buffer->GetMutableRowByOffset(2).EndOutput(17);
        buffer->GetMutableRowByOffset(4).ReplaceCharacters(0, 1, L"$");
        buffer->GetCursor().SetPosition({ 1, 4 });
        return buffer;
    }

    static void Compare(const TextBuffer& expected, const TextBuffer& actual)
    {
        SetVerifyOutput verifyOutput{ VerifyOutputSettings::LogOnlyFailures };
        VERIFY_IS_TRUE(expected.GetSize().Dimensions() == actual.GetSize().Dimensions());
        VERIFY_IS_TRUE(expected.GetCursor().GetPosition() == actual.GetCursor().GetPosition());
        const auto size = expected.GetSize().Dimensions();
        for (int y = 0; y < size.height; ++y)
        {
            const auto& a = expected.GetRowByOffset(y);
            const auto& b = actual.GetRowByOffset(y);
            VERIFY_IS_TRUE(a.WasWrapForced() == b.WasWrapForced());
            VERIFY_IS_TRUE(a.WasDoubleBytePadded() == b.WasDoubleBytePadded());
            VERIFY_IS_TRUE(a.GetLineRendition() == b.GetLineRendition());
            const auto& markA = a.GetScrollbarData();
            const auto& markB = b.GetScrollbarData();
            VERIFY_IS_TRUE(markA.has_value() == markB.has_value());
            if (markA && markB)
            {
                VERIFY_IS_TRUE(markA->category == markB->category);
                VERIFY_IS_TRUE(markA->color == markB->color);
                VERIFY_IS_TRUE(markA->exitCode == markB->exitCode);
            }
            for (int x = 0; x < size.width; ++x)
            {
                VERIFY_IS_TRUE(a.GlyphAt(x) == b.GlyphAt(x));
                VERIFY_IS_TRUE(a.DbcsAttrAt(x) == b.DbcsAttrAt(x));
                const auto attr = a.GetAttrByColumn(x);
                VERIFY_IS_TRUE(attr == b.GetAttrByColumn(x));
                if (attr.IsHyperlink())
                {
                    VERIFY_IS_TRUE(expected.GetHyperlinkUriFromId(attr.GetHyperlinkId()) == actual.GetHyperlinkUriFromId(attr.GetHyperlinkId()));
                }
            }
        }
    }

    static void Cover(TextBuffer& buffer)
    {
        for (int y = 0; y < 3; ++y)
        {
            buffer.GetMutableRowByOffset(y).Reset(TextAttribute{ 0x4f });
            buffer.GetMutableRowByOffset(y).ReplaceCharacters(0, 4, L"MENU");
        }
    }

    static std::unique_ptr<TextBuffer> Resize(TextBuffer& old, int width)
    {
        auto next = std::make_unique<TextBuffer>(til::size{ width, 64 }, TextAttribute{ 7 }, 0, false, &renderer);
        TextBuffer::Reflow(old, *next);
        return next;
    }

    TEST_METHOD(NativeFieldsAndForeignRowSurviveRestore)
    {
        auto reference = Fixture();
        auto overlaid = Fixture();
        NativeOverlaySnapshot snapshot{ *overlaid, 0, 3, 11 };
        Cover(*overlaid);
        // New input outside covered history must remain, including a new URI.
        for (auto* buffer : { reference.get(), overlaid.get() })
        {
            auto& row = buffer->GetMutableRowByOffset(4);
            row.ReplaceCharacters(1, 3, L"git");
            TextAttribute attr{ 7 };
            attr.SetHyperlinkId(buffer->GetHyperlinkId(L"https://example.invalid/new", L"new"));
            buffer->AddHyperlinkToMap(L"https://example.invalid/new", attr.GetHyperlinkId());
            row.ReplaceAttributes(1, 4, attr);
            buffer->GetCursor().SetPosition({ 4, 4 });
        }
        VERIFY_IS_TRUE(snapshot.RestoreBeforeResize(*overlaid, 11));
        Compare(*reference, *overlaid);
    }

    TEST_METHOD(RestoreBeforeRepeatedNativeReflowMatchesUncoveredBuffer)
    {
        auto reference = Fixture();
        auto overlaid = Fixture();
        // This compares against native Terminal reflow, not an independent
        // proof that Terminal reflow itself preserves every original field.
        for (const auto width : { 13, 40, 9, 24 })
        {
            NativeOverlaySnapshot snapshot{ *overlaid, 0, 3, 12 };
            Cover(*overlaid);
            VERIFY_IS_TRUE(snapshot.RestoreBeforeResize(*overlaid, 12));
            reference = Resize(*reference, width);
            overlaid = Resize(*overlaid, width);
            Compare(*reference, *overlaid);
        }
    }

    TEST_METHOD(StaleGenerationAndLinkCollisionFailBeforeMutation)
    {
        auto buffer = Fixture();
        NativeOverlaySnapshot snapshot{ *buffer, 0, 3, 13 };
        const auto link = buffer->GetRowByOffset(0).GetAttrByColumn(0).GetHyperlinkId();
        Cover(*buffer);
        VERIFY_IS_FALSE(snapshot.RestoreBeforeResize(*buffer, 14));
        buffer->AddHyperlinkToMap(L"https://example.invalid/reused-id", link);
        VERIFY_IS_FALSE(snapshot.RestoreBeforeResize(*buffer, 13));
        VERIFY_IS_TRUE(buffer->GetRowByOffset(0).GetText().substr(0, 4) == L"MENU");
    }

    TEST_METHOD(OriginalLogicalRowsRoundTripThroughRepeatedResize)
    {
        auto original = Fixture();
        auto resized = Fixture();
        for (const auto width : { 13, 40, 9, 24 })
        {
            resized = Resize(*resized, width);
        }
        // Independent comparison to the original, not to a second invocation
        // of the same reflow implementation. Any loss remains a release block.
        Compare(*original, *resized);
    }

    TEST_METHOD(ColoredTrailingBlankAttributesRoundTrip)
    {
        auto original = Fixture();
        auto resized = Fixture();
        for (auto* buffer : { original.get(), resized.get() })
        {
            for (int x = 20; x < 24; ++x)
            {
                // These spaces retain their distinct RGB/underline/link and
                // latent flags from Fixture; character-only trimming is lossy.
                buffer->GetMutableRowByOffset(2).ReplaceCharacters(x, 1, L" ");
            }
        }
        resized = Resize(*resized, 13);
        resized = Resize(*resized, 24);
        Compare(*original, *resized);
    }
};
