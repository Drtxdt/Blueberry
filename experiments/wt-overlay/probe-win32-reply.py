"""Dedicated-console diagnostic: can the reply arrive without VT input mode?

This establishes the direct bridge negotiation path before PSReadLine starts.
Never run it in a user's live editor or during formal timing.
"""
import ctypes
from ctypes import wintypes as w
import json
import secrets
import sys
import time
from pathlib import Path


class Key(ctypes.Structure):
    _fields_ = [("down", w.BOOL), ("repeat", w.WORD), ("key", w.WORD),
                ("scan", w.WORD), ("char", w.WCHAR), ("control", w.DWORD)]


class Record(ctypes.Structure):
    _fields_ = [("kind", w.WORD), ("padding", w.WORD), ("key", Key)]


def main():
    path = Path(sys.argv[1]).resolve()
    if path.exists():
        raise SystemExit("Refusing to overwrite input capability evidence")
    path.parent.mkdir(parents=True, exist_ok=True)
    result = {"schema": 1, "diagnostic_only": True, "supported": False,
              "input_kind": "ReadConsoleInputW", "input_mode_changed": False,
              "pixel_visibility_measured": False, "error": None}
    kernel = ctypes.WinDLL("kernel32", use_last_error=True)
    kernel.GetStdHandle.argtypes = [w.DWORD]
    kernel.GetStdHandle.restype = w.HANDLE
    kernel.GetConsoleMode.argtypes = [w.HANDLE, ctypes.POINTER(w.DWORD)]
    kernel.SetConsoleMode.argtypes = [w.HANDLE, w.DWORD]
    kernel.GetNumberOfConsoleInputEvents.argtypes = [w.HANDLE, ctypes.POINTER(w.DWORD)]
    kernel.ReadConsoleInputW.argtypes = [w.HANDLE, ctypes.POINTER(Record), w.DWORD, ctypes.POINTER(w.DWORD)]
    kernel.WriteFile.argtypes = [w.HANDLE, w.LPCVOID, w.DWORD, ctypes.POINTER(w.DWORD), w.LPVOID]
    stdin = kernel.GetStdHandle(-10 & 0xffffffff)
    stdout = kernel.GetStdHandle(-11 & 0xffffffff)
    input_mode, output_mode = w.DWORD(), w.DWORD()
    changed = False
    received = ""
    try:
        assert ctypes.sizeof(Record) == 20
        if not kernel.GetConsoleMode(stdin, ctypes.byref(input_mode)) or not kernel.GetConsoleMode(stdout, ctypes.byref(output_mode)):
            raise ctypes.WinError(ctypes.get_last_error())
        result["input_mode"] = input_mode.value
        if not kernel.SetConsoleMode(stdout, output_mode.value | 4):
            raise ctypes.WinError(ctypes.get_last_error())
        changed = True
        nonce = secrets.randbelow(2**63 - 1) + 1
        query = f"\x1b]7775;query;{nonce}\x07".encode("ascii")
        expected = f"\x1b]7775;prototype;1;{nonce};fixed-menu\x07"
        count = w.DWORD()
        if not kernel.WriteFile(stdout, query, len(query), ctypes.byref(count), None) or count.value != len(query):
            raise ctypes.WinError(ctypes.get_last_error())
        deadline = time.monotonic() + 8
        while time.monotonic() < deadline and len(received) < 4096:
            available = w.DWORD()
            if not kernel.GetNumberOfConsoleInputEvents(stdin, ctypes.byref(available)):
                raise ctypes.WinError(ctypes.get_last_error())
            if available.value:
                records = (Record * min(32, available.value))()
                if not kernel.ReadConsoleInputW(stdin, records, len(records), ctypes.byref(count)):
                    raise ctypes.WinError(ctypes.get_last_error())
                for record in records[:count.value]:
                    if record.kind == 1 and record.key.down and record.key.char != "\0":
                        received += record.key.char * min(record.key.repeat, 64)
                if expected in received:
                    result["supported"] = True
                    break
            else:
                time.sleep(.001)
        if not result["supported"]:
            result["error"] = "No matching reply within 8 seconds"
    except Exception as exc:
        result["error"] = str(exc)
    finally:
        result["received"] = received
        result["output_mode_restored"] = bool(kernel.SetConsoleMode(stdout, output_mode.value)) if changed else None
        path.write_text(json.dumps(result, ensure_ascii=False, indent=2) + "\n", encoding="utf-8")
    return 0 if result["supported"] else 1


if __name__ == "__main__":
    raise SystemExit(main())
