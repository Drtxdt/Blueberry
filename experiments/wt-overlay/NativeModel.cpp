// Copyright (c) Blueberry contributors. Licensed under the MIT license.
// Measurement-only C ABI over the same production TerminalCore static library.
// Native buffer observation is not a GUI renderer or a pixel timestamp.
#include "../TerminalCore/pch.h"
#include "../TerminalCore/Terminal.hpp"
#include "../../renderer/inc/DummyRenderer.hpp"
#include <mutex>
#include <vector>

using Microsoft::Terminal::Core::Terminal;

namespace
{
    using Response = void(__cdecl*)(void*, const wchar_t*, uint32_t);
    struct Model
    {
        std::mutex closedMutex;
        std::vector<std::wstring> closedOwners;
        Terminal terminal;
        DummyRenderer renderer{ &terminal };
        std::string pendingUtf8;

        Model(uint16_t rows, uint16_t columns, uint16_t history, Response response, void* context)
        {
            const auto lock = terminal.LockForWriting();
            terminal.Create({ columns, rows }, history, renderer);
            terminal.SetWriteInputCallback([response, context](std::wstring_view text) {
                if (response) { response(context, text.data(), static_cast<uint32_t>(text.size())); }
            });
            terminal.SetNativeOverlayClosedCallback([this](std::wstring owner) {
                const std::lock_guard guard{ closedMutex };
                closedOwners.push_back(std::move(owner));
            });
        }

        // The real GUI uses its dispatcher. This buffer-only observer drains
        // owner-bound EOF notifications at its explicit read/write boundaries.
        void DrainClosed()
        {
            std::vector<std::wstring> owners;
            {
                const std::lock_guard guard{ closedMutex };
                owners.swap(closedOwners);
            }
            for (const auto& owner : owners) { terminal.RetireNativeOverlayChannel(owner); }
        }
    };
}

extern "C" __declspec(dllexport) uint32_t __cdecl bb_native_model_version() noexcept { return 1; }
extern "C" __declspec(dllexport) uint32_t __cdecl bb_native_model_overlay_version() noexcept { return 4; }

extern "C" __declspec(dllexport) void* __cdecl bb_native_model_create(
    uint16_t rows, uint16_t columns, uint16_t history, Response response, void* context) noexcept
{
    try
    {
        if (rows < 4 || rows > 512 || columns < 8 || columns > 512 || history > 30000) { return nullptr; }
        return new Model{ rows, columns, history, response, context };
    }
    catch (...) { return nullptr; }
}

extern "C" __declspec(dllexport) void __cdecl bb_native_model_destroy(void* value) noexcept
{
    delete static_cast<Model*>(value);
}

extern "C" __declspec(dllexport) HRESULT __cdecl bb_native_model_write(void* value, const char* data, uint32_t length) noexcept
try
{
    if (!value || (!data && length) || length > 1024 * 1024) { return E_INVALIDARG; }
    auto& model = *static_cast<Model*>(value);
    const auto lock = model.terminal.LockForWriting();
    model.DrainClosed();
    if (length) { model.pendingUtf8.append(data, length); }
    if (model.pendingUtf8.empty()) { return S_OK; }
    const auto incompleteTail = [&](size_t begin) {
        const auto lead = static_cast<unsigned char>(model.pendingUtf8[begin]);
        const auto expected = lead >= 0xc2 && lead <= 0xdf ? 2u : lead >= 0xe0 && lead <= 0xef ? 3u : lead >= 0xf0 && lead <= 0xf4 ? 4u : 0u;
        if (expected <= model.pendingUtf8.size() - begin) { return false; }
        for (size_t i = begin + 1; i < model.pendingUtf8.size(); ++i)
        {
            const auto ch = static_cast<unsigned char>(model.pendingUtf8[i]);
            if ((ch & 0xc0) != 0x80) { return false; }
            if (i == begin + 1 && ((lead == 0xe0 && ch < 0xa0) || (lead == 0xed && ch >= 0xa0) ||
                                  (lead == 0xf0 && ch < 0x90) || (lead == 0xf4 && ch >= 0x90))) { return false; }
        }
        return true;
    };
    // ConPTY emits UTF-8, but its read boundary may split a scalar. Retain at
    // most the incomplete final scalar; malformed input invalidates collection.
    for (size_t tail = 0; tail <= 3 && tail <= model.pendingUtf8.size(); ++tail)
    {
        const auto complete = model.pendingUtf8.size() - tail;
        if (tail && !incompleteTail(complete)) { continue; }
        if (!complete) { return S_FALSE; }
        const auto size = MultiByteToWideChar(CP_UTF8, MB_ERR_INVALID_CHARS, model.pendingUtf8.data(),
                                             static_cast<int>(complete), nullptr, 0);
        if (!size) { continue; }
        std::wstring text(static_cast<size_t>(size), L'\0');
        if (!MultiByteToWideChar(CP_UTF8, MB_ERR_INVALID_CHARS, model.pendingUtf8.data(),
                                 static_cast<int>(complete), text.data(), size)) { return E_FAIL; }
        model.pendingUtf8.erase(0, complete);
        model.terminal.Write(text);
        return S_OK;
    }
    return HRESULT_FROM_WIN32(ERROR_NO_UNICODE_TRANSLATION);
}
CATCH_RETURN();

extern "C" __declspec(dllexport) HRESULT __cdecl bb_native_model_resize(void* value, uint16_t rows, uint16_t columns) noexcept
try
{
    if (!value || rows < 4 || rows > 512 || columns < 8 || columns > 512) { return E_INVALIDARG; }
    auto& model = *static_cast<Model*>(value);
    const auto lock = model.terminal.LockForWriting();
    model.DrainClosed();
    return model.terminal.UserResize({ columns, rows });
}
CATCH_RETURN();

extern "C" __declspec(dllexport) HRESULT __cdecl bb_native_model_snapshot(
    void* value, wchar_t* output, uint32_t capacity, uint32_t* needed) noexcept
try
{
    if (!value || !needed || (!output && capacity)) { return E_INVALIDARG; }
    auto& model = *static_cast<Model*>(value);
    const auto lock = model.terminal.LockForWriting();
    model.DrainClosed();
    const auto viewport = model.terminal.GetViewport();
    const auto& buffer = model.terminal.GetTextBuffer();
    std::wstring text;
    for (auto y = viewport.Top(); y < viewport.BottomExclusive(); ++y)
    {
        if (!text.empty()) { text.push_back(L'\n'); }
        text.append(buffer.GetRowByOffset(y).GetText());
    }
    if (text.size() >= 2 * 1024 * 1024) { return E_BOUNDS; }
    *needed = static_cast<uint32_t>(text.size()) + 1;
    if (capacity < *needed) { return HRESULT_FROM_WIN32(ERROR_INSUFFICIENT_BUFFER); }
    std::copy(text.begin(), text.end(), output);
    output[text.size()] = L'\0';
    return S_OK;
}
CATCH_RETURN();
