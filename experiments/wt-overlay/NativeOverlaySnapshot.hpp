// Copyright (c) Blueberry contributors. Licensed under the MIT license.
#pragma once

#include "textBuffer.hpp"

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
        _saved{ _size, TextAttribute{}, 0, false, nullptr }
    {
        if (begin < 0 || begin >= end || end > _size.height)
        {
            throw std::invalid_argument("overlay row range");
        }
        _saved.CopyHyperlinkMaps(source);
        for (auto y = begin; y < end; ++y)
        {
            CopyCompleteRow(source, _saved, y);
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
        for (auto y = _begin; y < _end; ++y)
        {
            for (const auto id : _saved.GetRowByOffset(y).GetHyperlinks())
            {
                try
                {
                    if (target.GetHyperlinkUriFromId(id) != _saved.GetHyperlinkUriFromId(id))
                    {
                        return false;
                    }
                }
                catch (const std::out_of_range&)
                {
                    return false;
                }
            }
        }
        for (auto y = _begin; y < _end; ++y)
        {
            CopyCompleteRow(_saved, target, y);
        }
        return true;
    }

private:
    static void CopyCompleteRow(const TextBuffer& source, TextBuffer& target,
                                til::CoordType y)
    {
        source.CopyRow(y, y, target);
        const auto& src = source.GetRowByOffset(y);
        auto& dst = target.GetMutableRowByOffset(y);
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
};
