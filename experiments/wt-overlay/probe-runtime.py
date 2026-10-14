"""Dedicated-console, bounded ConPTY capability probe; never a pixel/restore gate.

Run from a separate, unpackaged prototype Terminal, with no shell profile.
Console modes are restored; no settings, installation or clipboard is touched.
"""
import argparse
import ctypes
from ctypes import wintypes
import json
import os
from pathlib import Path
import secrets
import threading
import time


def main():
    parser = argparse.ArgumentParser()
    parser.add_argument("--output", type=Path, required=True)
    args = parser.parse_args()
    if args.output.exists():
        raise SystemExit("Refusing to overwrite runtime evidence")
    args.output.parent.mkdir(parents=True, exist_ok=True)
    result = {"schema": 1, "diagnostic_only": True,
              "source": "own_dedicated_console_input", "supported": False,
              "pixel_visibility_measured": False,
              "complete_history_restoration_verified": False,
              "wt_session": os.environ.get("WT_SESSION"), "error": None}
    kernel = ctypes.WinDLL("kernel32", use_last_error=True)
    kernel.GetStdHandle.argtypes = [wintypes.DWORD]
    kernel.GetStdHandle.restype = wintypes.HANDLE
    kernel.GetConsoleMode.argtypes = [wintypes.HANDLE, ctypes.POINTER(wintypes.DWORD)]
    kernel.SetConsoleMode.argtypes = [wintypes.HANDLE, wintypes.DWORD]
    kernel.ReadFile.argtypes = [wintypes.HANDLE, wintypes.LPVOID, wintypes.DWORD,
                               ctypes.POINTER(wintypes.DWORD), wintypes.LPVOID]
    kernel.WriteFile.argtypes = kernel.ReadFile.argtypes
    kernel.OpenThread.argtypes = [wintypes.DWORD, wintypes.BOOL, wintypes.DWORD]
    kernel.OpenThread.restype = wintypes.HANDLE
    kernel.CancelSynchronousIo.argtypes = [wintypes.HANDLE]
    kernel.CloseHandle.argtypes = [wintypes.HANDLE]
    stdin = kernel.GetStdHandle(-10 & 0xffffffff)
    stdout = kernel.GetStdHandle(-11 & 0xffffffff)
    in_mode, out_mode = wintypes.DWORD(), wintypes.DWORD()
    changed = False
    reader = None
    thread_id = []
    received = bytearray()
    ready = threading.Event()
    done = threading.Event()
    nonce = secrets.randbelow(2**63 - 1) + 1
    expected = f"\x1b]7775;prototype;1;{nonce};fixed-menu\x07".encode("ascii")

    def write(data):
        count = wintypes.DWORD()
        if not kernel.WriteFile(stdout, data, len(data), ctypes.byref(count), None):
            raise ctypes.WinError(ctypes.get_last_error())
        if count.value != len(data):
            raise RuntimeError("Short console output write")

    def read_reply():
        thread_id.append(kernel.GetCurrentThreadId())
        ready.set()
        try:
            while len(received) < 4096:
                buf = ctypes.create_string_buffer(512)
                count = wintypes.DWORD()
                if not kernel.ReadFile(stdin, buf, len(buf), ctypes.byref(count), None):
                    result["read_error"] = ctypes.get_last_error()
                    break
                if not count.value:
                    break
                received.extend(buf.raw[:count.value])
                if expected in received:
                    break
        finally:
            done.set()

    try:
        if not kernel.GetConsoleMode(stdin, ctypes.byref(in_mode)):
            raise ctypes.WinError(ctypes.get_last_error())
        if not kernel.GetConsoleMode(stdout, ctypes.byref(out_mode)):
            raise ctypes.WinError(ctypes.get_last_error())
        if not kernel.SetConsoleMode(stdin, (in_mode.value | 0x200) & ~7):
            raise ctypes.WinError(ctypes.get_last_error())
        changed = True
        if not kernel.SetConsoleMode(stdout, out_mode.value | 4):
            raise ctypes.WinError(ctypes.get_last_error())
        reader = threading.Thread(target=read_reply, daemon=True)
        reader.start()
        if not ready.wait(2):
            raise RuntimeError("Input reader did not start")
        start = time.monotonic()
        # Query only: no draw command, cursor movement, clipboard or history edits.
        write(f"\x1b]7775;query;{nonce}\x07".encode("ascii"))
        done.wait(8)
        result["supported"] = expected in received
        result["query_roundtrip_ms"] = (time.monotonic() - start) * 1000
        if not result["supported"]:
            result["error"] = "Expected nonce-bound prototype response not received within 8 seconds"
    except Exception as exc:
        result["error"] = str(exc)
    finally:
        if reader is not None and reader.is_alive() and thread_id:
            handle = kernel.OpenThread(1, False, thread_id[0])
            if handle:
                kernel.CancelSynchronousIo(handle)
                kernel.CloseHandle(handle)
            reader.join(2)
        result["reader_cleaned_up"] = reader is None or not reader.is_alive()
        result["received_ascii"] = bytes(received).decode("ascii", errors="backslashreplace")
        if changed:
            in_ok = kernel.SetConsoleMode(stdin, in_mode.value)
            out_ok = kernel.SetConsoleMode(stdout, out_mode.value)
            result["console_modes_restored"] = bool(in_ok and out_ok)
        args.output.write_text(json.dumps(result, ensure_ascii=False, indent=2) + "\n", encoding="utf-8")
    return 0 if result["supported"] and result["reader_cleaned_up"] else 1


if __name__ == "__main__":
    raise SystemExit(main())
