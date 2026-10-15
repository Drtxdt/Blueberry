// Copyright (c) Blueberry contributors. Licensed under the MIT license.
#pragma once

#include "textBuffer.hpp"
#include <algorithm>
#include <string>
#include <vector>

// Feasibility primitive, not a production VT interface. The owner must call
// RestoreBeforeResize under the Terminal write lock BEFORE native reflow.
// Overlapping foreign writes, scrolling, hyperlink pruning and renderer
// integration are deliberately not claimed to be supported by this prototype.
class NativeOverlaySnapshot final
{
public:
    NativeOverlaySnapshot(const TextBuffer& source, til::CoordType begin,
                          til::CoordType end, uint64_t generation) :
        _size{ source.GetSize().Dimensions() },
        _begin{ begin }, _end{ end }, _generation{ generation },
        _saved{ CoveredSize(_size, begin, end), TextAttribute{}, 0, false, nullptr }
    {
        for (auto y = begin; y < end; ++y)
        {
            CopyCompleteRow(source, y, _saved, y - begin);
            for (const auto id : source.GetRowByOffset(y).GetHyperlinks())
            {
                if (std::none_of(_links.begin(), _links.end(), [id](const auto& link) { return link.id == id; }))
                {
                    _links.push_back({ id, source.GetHyperlinkUriFromId(id), source.GetCustomIdFromId(id) });
                }
            }
        }
    }

    bool RestoreBeforeResize(TextBuffer& target, uint64_t generation) const
    {
        if (generation != _generation || target.GetSize().Dimensions() != _size)
        {
            return false;
        }
        // Never replace the entire live hyperlink map: it may contain new URIs.
        // A production implementation needs ID lifetime pinning/remapping.
        // Fail before any row mutation if the original ID was pruned or reused.
        for (const auto& link : _links)
        {
            try
            {
                if (target.GetHyperlinkUriFromId(link.id) != link.uri ||
                    target.GetCustomIdFromId(link.id) != link.customId)
                {
                    return false;
                }
            }
            catch (const std::out_of_range&)
            {
                return false;
            }
        }
        for (auto y = _begin; y < _end; ++y)
        {
            CopyCompleteRow(_saved, y - _begin, target, y);
        }
        return true;
    }

private:
    static til::size CoveredSize(til::size size, til::CoordType begin, til::CoordType end)
    {
        if (begin < 0 || begin >= end || end > size.height)
        {
            throw std::invalid_argument("overlay row range");
        }
        return { size.width, end - begin };
    }

    static void CopyCompleteRow(const TextBuffer& source, til::CoordType sourceY,
                                TextBuffer& target, til::CoordType targetY)
    {
        source.CopyRow(sourceY, targetY, target);
        const auto& src = source.GetRowByOffset(sourceY);
        auto& dst = target.GetMutableRowByOffset(targetY);
        // CopyRow follows rendered width. Preserve physical columns too: a
        // double-width/height row can retain hidden character storage there.
        RowCopyTextFromState text{
            .source = src,
            .columnBegin = 0,
            .columnLimit = til::CoordTypeMax,
            .sourceColumnBegin = 0,
            .sourceColumnLimit = src.size(),
        };
        dst.CopyTextFrom(text);
        dst.SetDoubleBytePadded(src.WasDoubleBytePadded());
        dst.SetScrollbarData(src.GetScrollbarData());
    }

    til::size _size;
    til::CoordType _begin;
    til::CoordType _end;
    uint64_t _generation;
    TextBuffer _saved;
    struct Link { uint16_t id; std::wstring uri; std::wstring customId; };
    std::vector<Link> _links;
};
