// Copyright (c) Blueberry contributors. Licensed under the MIT license.
#pragma once

#include <atomic>
#include <deque>
#include <functional>
#include <mutex>
#include <string>
#include <thread>

// Replies never enter the application's keyboard stream. One bounded,
// event-driven worker owns overlapped pipe I/O; destruction cancels and joins.
// The callback must enqueue retirement on the owner's dispatcher, not acquire
// the Terminal write lock on this worker.
class NativeOverlayChannel final
{
public:
    static std::unique_ptr<NativeOverlayChannel> Connect(
        std::wstring_view owner, DWORD serverPid, std::function<void()> closed)
    {
        if (owner.size() != 32 || serverPid == 0) { return nullptr; }
        for (const auto ch : owner)
        {
            if (!((ch >= L'0' && ch <= L'9') || (ch >= L'a' && ch <= L'f'))) { return nullptr; }
        }
        const auto name = L"\\\\.\\pipe\\Blueberry.TerminalOverlay." + std::wstring{ owner };
        wil::unique_hfile pipe{ CreateFileW(name.c_str(), GENERIC_READ | GENERIC_WRITE,
            0, nullptr, OPEN_EXISTING, FILE_FLAG_OVERLAPPED | SECURITY_SQOS_PRESENT | SECURITY_IDENTIFICATION, nullptr) };
        if (!pipe) { return nullptr; }
        ULONG actualPid = 0;
        if (!GetNamedPipeServerProcessId(pipe.get(), &actualPid) || actualPid != serverPid) { return nullptr; }
        return std::unique_ptr<NativeOverlayChannel>{ new NativeOverlayChannel(std::move(pipe), owner, std::move(closed)) };
    }

    ~NativeOverlayChannel()
    {
        SetEvent(_stop.get());
        if (_worker.joinable()) { _worker.join(); }
    }

    bool Send(std::string message)
    {
        if (message.empty() || message.size() > 192 || message.back() != '\n' || !_connected.load()) { return false; }
        {
            const std::lock_guard lock{ _mutex };
            if (_queue.size() >= 64)
            {
                // Fail closed: never acknowledge a frame after losing ordering.
                _connected.store(false);
                SetEvent(_queueReady.get());
                return false;
            }
            _queue.push_back(std::move(message));
        }
        SetEvent(_queueReady.get());
        return true;
    }

    bool Connected() const noexcept { return _connected.load(); }
    const std::wstring& Owner() const noexcept { return _owner; }

private:
    NativeOverlayChannel(wil::unique_hfile pipe, std::wstring_view owner, std::function<void()> closed) :
        _pipe{ std::move(pipe) }, _owner{ owner }, _closed{ std::move(closed) },
        _stop{ CreateEventW(nullptr, TRUE, FALSE, nullptr) },
        _queueReady{ CreateEventW(nullptr, FALSE, FALSE, nullptr) }
    {
        THROW_LAST_ERROR_IF(!_stop || !_queueReady);
        std::string asciiOwner;
        asciiOwner.reserve(owner.size());
        for (const auto ch : owner) { asciiOwner.push_back(static_cast<char>(ch)); }
        _queue.push_back("ready;4;" + asciiOwner + "\n");
        SetEvent(_queueReady.get());
        _worker = std::thread{ [this]() noexcept { Run(); } };
    }

    void Run() noexcept
    {
        try { Transfer(); } catch (...) { }
        _pipe.reset();
        _connected.store(false);
        if (WaitForSingleObject(_stop.get(), 0) != WAIT_OBJECT_0 && _closed)
        {
            try { _closed(); } catch (...) { /* Never escape a native I/O thread. */ }
        }
    }

    void Transfer()
    {
        wil::unique_handle readEvent{ CreateEventW(nullptr, TRUE, FALSE, nullptr) };
        wil::unique_handle writeEvent{ CreateEventW(nullptr, TRUE, FALSE, nullptr) };
        if (readEvent && writeEvent)
        {
            OVERLAPPED read{};
            read.hEvent = readEvent.get();
            char unexpected{};
            DWORD count{};
            const bool pending = !ReadFile(_pipe.get(), &unexpected, 1, &count, &read) && GetLastError() == ERROR_IO_PENDING;
            if (pending)
            {
                const auto cancelRead = wil::scope_exit([&]() noexcept {
                    CancelIoEx(_pipe.get(), &read);
                    GetOverlappedResult(_pipe.get(), &read, &count, TRUE);
                });
                const HANDLE waits[]{ _stop.get(), readEvent.get(), _queueReady.get() };
                while (_connected.load())
                {
                    const auto event = WaitForMultipleObjects(3, waits, FALSE, INFINITE);
                    if (event != WAIT_OBJECT_0 + 2) { break; }
                    while (_connected.load())
                    {
                        std::string message;
                        {
                            const std::lock_guard lock{ _mutex };
                            if (_queue.empty()) { break; }
                            message = std::move(_queue.front());
                            _queue.pop_front();
                        }
                        OVERLAPPED write{};
                        write.hEvent = writeEvent.get();
                        ResetEvent(writeEvent.get());
                        bool written = WriteFile(_pipe.get(), message.data(), static_cast<DWORD>(message.size()), &count, &write) != FALSE;
                        if (!written && GetLastError() == ERROR_IO_PENDING)
                        {
                            const HANDLE writeWaits[]{ _stop.get(), readEvent.get(), writeEvent.get() };
                            written = WaitForMultipleObjects(3, writeWaits, FALSE, 2000) == WAIT_OBJECT_0 + 2 &&
                                      GetOverlappedResult(_pipe.get(), &write, &count, FALSE);
                            if (!written)
                            {
                                CancelIoEx(_pipe.get(), &write);
                                GetOverlappedResult(_pipe.get(), &write, &count, TRUE);
                            }
                        }
                        if (!written || count != message.size()) { _connected.store(false); break; }
                    }
                }
            }
        }
    }

    wil::unique_hfile _pipe;
    std::wstring _owner;
    std::function<void()> _closed;
    wil::unique_handle _stop, _queueReady;
    std::atomic<bool> _connected{ true };
    std::mutex _mutex;
    std::deque<std::string> _queue;
    std::thread _worker;
};
