// Copyright (c) Blueberry contributors. Licensed under the MIT license.
#pragma once
#include "textBuffer.hpp"

// A bounded menu payload, not a general VT execution channel. Only printable
// UTF-16, row separators and the SGR emitted by Blueberry's menu renderer pass.
// Build the entire frame off-screen before retiring any current overlay.
class NativeOverlayFrame final
{
public:
    static std::unique_ptr<TextBuffer> Parse(std::wstring_view hex, til::CoordType width, til::CoordType height,
                                           const TextAttribute& initialAttributes)
    {
        if (width < 1 || width > 512 || height < 1 || height > 512 ||
            hex.empty() || hex.size() > 262144 || hex.size() % 4 != 0)
        {
            return nullptr;
        }
        std::wstring text;
        text.reserve(hex.size() / 4);
        for (size_t i = 0; i < hex.size(); i += 4)
        {
            unsigned value = 0;
            for (size_t j = 0; j < 4; ++j)
            {
                const auto c = hex[i + j];
                const auto digit = c >= L'0' && c <= L'9' ? c - L'0' :
                                   c >= L'a' && c <= L'f' ? c - L'a' + 10 :
                                   c >= L'A' && c <= L'F' ? c - L'A' + 10 : -1;
                if (digit < 0) { return nullptr; }
                value = value * 16 + static_cast<unsigned>(digit);
            }
            text.push_back(static_cast<wchar_t>(value));
        }
        for (size_t i = 0; i < text.size(); ++i)
        {
            const auto c = text[i];
            if (c >= 0xd800 && c <= 0xdbff)
            {
                if (++i == text.size() || text[i] < 0xdc00 || text[i] > 0xdfff) { return nullptr; }
            }
            else if (c >= 0xdc00 && c <= 0xdfff) { return nullptr; }
        }
        auto buffer = std::make_unique<TextBuffer>(til::size{ width, height }, TextAttribute{}, 0, false, nullptr);
        til::CoordType rowIndex = 0;
        til::CoordType column = 0;
        // Match ordinary VT: text before its first SGR inherits current state.
        // The client includes each existing menu row's terminating SGR reset.
        TextAttribute attr = initialAttributes;
        size_t pos = 0;
        while (pos < text.size())
        {
            if (text[pos] == L'\n')
            {
                if (++rowIndex >= height) { return nullptr; }
                column = 0;
                ++pos;
                continue;
            }
            if (text[pos] == L'\x1b')
            {
                if (pos + 1 >= text.size() || text[pos + 1] != L'[') { return nullptr; }
                const auto end = text.find(L'm', pos + 2);
                if (end == std::wstring::npos || end - pos > 64) { return nullptr; }
                std::vector<unsigned> codes;
                unsigned value = 0;
                bool digit = false;
                for (auto i = pos + 2; i < end; ++i)
                {
                    const auto c = text[i];
                    if (c >= L'0' && c <= L'9')
                    {
                        value = value * 10 + static_cast<unsigned>(c - L'0');
                        if (value > 255) { return nullptr; }
                        digit = true;
                    }
                    else if (c == L';' && digit)
                    {
                        codes.push_back(value); value = 0; digit = false;
                    }
                    else { return nullptr; }
                }
                if (!digit) { return nullptr; }
                codes.push_back(value);
                for (size_t i = 0; i < codes.size(); ++i)
                {
                    const auto code = codes[i];
                    if (code == 0)
                    {
                        // SGR 0 does not close OSC 8 or reset protection/marks.
                        attr.SetDefaultForeground();
                        attr.SetDefaultBackground();
                        attr.SetDefaultUnderlineColor();
                        attr.SetDefaultRenditionAttributes();
                    }
                    else if (code == 1) { attr.SetIntense(true); }
                    else if (code == 22) { attr.SetIntense(false); attr.SetFaint(false); }
                    else if ((code == 38 || code == 48) && i + 4 < codes.size() && codes[i + 1] == 2)
                    {
                        const auto rgb = RGB(codes[i + 2], codes[i + 3], codes[i + 4]);
                        if (code == 38) { attr.SetForeground(rgb); }
                        else { attr.SetBackground(rgb); }
                        i += 4;
                    }
                    else { return nullptr; }
                }
                pos = end + 1;
                continue;
            }
            const auto begin = pos;
            while (pos < text.size() && text[pos] != L'\n' && text[pos] != L'\x1b')
            {
                if (text[pos] < 0x20 || (text[pos] >= 0x7f && text[pos] <= 0x9f)) { return nullptr; }
                ++pos;
            }
            auto& row = buffer->GetMutableRowByOffset(rowIndex);
            RowWriteState state{ .text = std::wstring_view{ text }.substr(begin, pos - begin),
                                 .columnBegin = column, .columnLimit = width };
            row.ReplaceText(state);
            if (!state.text.empty()) { return nullptr; }
            row.ReplaceAttributes(state.columnBeginDirty, state.columnEndDirty, attr);
            column = state.columnEnd;
        }
        if (rowIndex + 1 != height) { return nullptr; }
        return buffer;
    }

    static void Paint(const TextBuffer& frame, TextBuffer& target, til::CoordType top, til::CoordType left = 0)
    {
        const auto width = frame.GetSize().Width();
        for (til::CoordType y = 0; y < frame.GetSize().Height(); ++y)
        {
            const auto& src = frame.GetRowByOffset(y);
            auto& dst = target.GetMutableRowByOffset(top + y);
            RowCopyTextFromState text{ .source = src, .columnBegin = left, .columnLimit = left + width,
                                      .sourceColumnBegin = 0, .sourceColumnLimit = width };
            dst.CopyTextFrom(text);
            for (til::CoordType x = 0; x < width; ++x)
            {
                dst.ReplaceAttributes(left + x, left + x + 1, src.GetAttrByColumn(x));
            }
        }
        target.TriggerRedrawAll();
    }
};
