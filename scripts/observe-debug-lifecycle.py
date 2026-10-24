"""Record owned Windows process lifecycle events using DEBUG_PROCESS.

This is a separate diagnostic run, never a source of performance timings.
Descendants that establish their own debugger and processes launched through
an external service/broker are outside this observer's documented scope.
https://learn.microsoft.com/windows/win32/debug/debugging-events
"""
import argparse
import ctypes as c
from ctypes import wintypes as w
import importlib.util
import json
import os
from pathlib import Path
import subprocess
import time
from types import SimpleNamespace


class ExceptionRecord(c.Structure):
    _fields_ = [('code', w.DWORD), ('flags', w.DWORD), ('record', c.c_void_p),
                ('address', c.c_void_p), ('count', w.DWORD), ('parameters', c.c_size_t * 15)]


class ExceptionInfo(c.Structure):
    _fields_ = [('record', ExceptionRecord), ('first_chance', w.DWORD)]


class CreateInfo(c.Structure):
    _fields_ = [('file', w.HANDLE), ('process', w.HANDLE), ('thread', w.HANDLE),
                ('base', c.c_void_p), ('debug_offset', w.DWORD), ('debug_size', w.DWORD),
                ('local_base', c.c_void_p), ('start', c.c_void_p), ('image_name', c.c_void_p),
                ('unicode', w.WORD)]


class DebugData(c.Union):
    _fields_ = [('exception', ExceptionInfo), ('create', CreateInfo),
                ('exit_code', w.DWORD), ('file', w.HANDLE)]


class DebugEvent(c.Structure):
    _fields_ = [('kind', w.DWORD), ('pid', w.DWORD), ('tid', w.DWORD), ('data', DebugData)]


class ProcessEntry(c.Structure):
    _fields_ = [('size', w.DWORD), ('usage', w.DWORD), ('pid', w.DWORD),
                ('heap', c.c_size_t), ('module', w.DWORD), ('threads', w.DWORD),
                ('parent', w.DWORD), ('priority', w.LONG), ('flags', w.DWORD),
                ('name', w.WCHAR * 260)]


def configure():
    kernel = c.WinDLL('kernel32', use_last_error=True)
    signatures = {
        'WaitForDebugEvent': ([c.POINTER(DebugEvent), w.DWORD], w.BOOL),
        'ContinueDebugEvent': ([w.DWORD, w.DWORD, w.DWORD], w.BOOL),
        'ResumeThread': ([w.HANDLE], w.DWORD),
        'CloseHandle': ([w.HANDLE], w.BOOL),
        'QueryFullProcessImageNameW': ([w.HANDLE, w.DWORD, w.LPWSTR, c.POINTER(w.DWORD)], w.BOOL),
        'GetProcessTimes': ([w.HANDLE] + [c.POINTER(w.FILETIME)] * 4, w.BOOL),
        'CreateToolhelp32Snapshot': ([w.DWORD, w.DWORD], w.HANDLE),
        'Process32FirstW': ([w.HANDLE, c.POINTER(ProcessEntry)], w.BOOL),
        'Process32NextW': ([w.HANDLE, c.POINTER(ProcessEntry)], w.BOOL),
    }
    for name, (args, result) in signatures.items():
        function = getattr(kernel, name)
        function.argtypes, function.restype = args, result
    return kernel


def parent_pid(kernel, pid):
    snapshot = kernel.CreateToolhelp32Snapshot(2, 0)
    if snapshot == c.c_void_p(-1).value:
        raise c.WinError(c.get_last_error())
    try:
        entry = ProcessEntry()
        entry.size = c.sizeof(entry)
        found = kernel.Process32FirstW(snapshot, c.byref(entry))
        while found:
            if entry.pid == pid:
                return entry.parent
            found = kernel.Process32NextW(snapshot, c.byref(entry))
        raise RuntimeError(f'Parent unavailable for paused process {pid}')
    finally:
        kernel.CloseHandle(snapshot)


def process_record(kernel, event):
    handle = event.data.create.process
    size = w.DWORD(32768)
    image = c.create_unicode_buffer(size.value)
    if not kernel.QueryFullProcessImageNameW(handle, 0, image, c.byref(size)):
        raise c.WinError(c.get_last_error())
    times = [w.FILETIME() for _ in range(4)]
    if not kernel.GetProcessTimes(handle, *(c.byref(value) for value in times)):
        raise c.WinError(c.get_last_error())
    return dict(pid=event.pid, parent_pid=parent_pid(kernel, event.pid), image=image.value,
                created_filetime=times[0].dwLowDateTime + (times[0].dwHighDateTime << 32),
                observed_start_ns=time.monotonic_ns(), observed_exit_ns=None, exit_code=None)


def observe(argv, output, timeout):
    if os.name != 'nt':
        raise OSError('DEBUG_PROCESS observer requires Windows')
    import _winapi
    import msvcrt
    output = Path(output).resolve()
    output.mkdir(parents=True, exist_ok=False)
    kernel = configure()
    source = Path(__file__).with_name('run-benchmark-batch.py')
    spec = importlib.util.spec_from_file_location('owned_batch', source)
    batch = importlib.util.module_from_spec(spec)
    spec.loader.exec_module(batch)
    receipt = dict(schema=1, diagnostic_only=True, formal_timing_eligible=False,
                   observer='Windows DEBUG_PROCESS', observer_sha256=batch.sha(__file__),
                   argv=argv, scope='Owned debug-inherited descendants; excludes service/broker launches and descendants selecting their own debugger',
                   lifecycle_reconciled=False, workload_succeeded=False, workload_exit_code=None,
                   processes=[], error=None)
    live = {}
    initial_breakpoint = set()
    process_handle = thread_handle = None
    job = None
    assigned = False
    try:
        receipt['executable_sha256'] = batch.sha(argv[0])
        with (output / 'stdout.log').open('wb') as stdout, (output / 'stderr.log').open('wb') as stderr, open(os.devnull, 'rb') as stdin:
            handles = [msvcrt.get_osfhandle(stream.fileno()) for stream in (stdin, stdout, stderr)]
            for handle in handles:
                os.set_handle_inheritable(handle, True)
            startup = subprocess.STARTUPINFO()
            startup.dwFlags = subprocess.STARTF_USESTDHANDLES | subprocess.STARTF_USESHOWWINDOW
            startup.wShowWindow = 0
            startup.hStdInput, startup.hStdOutput, startup.hStdError = handles
            try:
                process_handle, thread_handle, root_pid, _ = _winapi.CreateProcess(
                    argv[0], subprocess.list2cmdline(argv), None, None, True,
                    0x1 | 0x4 | 0x08000000, None, None, startup)
            finally:
                for handle in handles:
                    os.set_handle_inheritable(handle, False)
            receipt['root_pid'] = root_pid
            job = batch.WindowsJob()
            job.assign(SimpleNamespace(_handle=process_handle))
            assigned = True
            if kernel.ResumeThread(thread_handle) == 0xFFFFFFFF:
                raise c.WinError(c.get_last_error())
            deadline = time.monotonic() + timeout
            with (output / 'events.jsonl').open('w', encoding='utf-8') as stream:
                while True:
                    if time.monotonic() >= deadline:
                        raise TimeoutError('Owned lifecycle diagnostic timed out')
                    event = DebugEvent()
                    if not kernel.WaitForDebugEvent(c.byref(event), 100):
                        if c.get_last_error() == 121:
                            continue
                        raise c.WinError(c.get_last_error())
                    disposition = 0x00010002  # DBG_CONTINUE
                    try:
                        detail = dict(kind=event.kind, pid=event.pid, tid=event.tid,
                                      observed_ns=time.monotonic_ns())
                        if event.kind == 3:  # CREATE_PROCESS_DEBUG_EVENT
                            if event.data.create.file:
                                kernel.CloseHandle(event.data.create.file)
                            if event.pid in live:
                                raise RuntimeError('Duplicate live PID')
                            record = process_record(kernel, event)
                            parent = next((row for row in reversed(receipt['processes'])
                                           if row['pid'] == record['parent_pid']), None)
                            record['parent_created_filetime'] = parent['created_filetime'] if parent else None
                            live[event.pid] = record
                            receipt['processes'].append(record)
                            detail['process'] = dict(record)
                        elif event.kind == 5:  # EXIT_PROCESS_DEBUG_EVENT
                            record = live.pop(event.pid)
                            record.update(observed_exit_ns=detail['observed_ns'], exit_code=event.data.exit_code)
                            detail['exit_code'] = event.data.exit_code
                            if event.pid == root_pid:
                                receipt['workload_exit_code'] = event.data.exit_code
                                receipt['workload_succeeded'] = event.data.exit_code == 0
                        elif event.kind == 6 and event.data.file:  # LOAD_DLL_DEBUG_EVENT
                            kernel.CloseHandle(event.data.file)
                        elif event.kind == 1:
                            code = event.data.exception.record.code
                            detail.update(exception_code=code, first_chance=event.data.exception.first_chance)
                            if code == 0x80000003 and event.pid not in initial_breakpoint:
                                initial_breakpoint.add(event.pid)
                            else:
                                disposition = 0x80010001  # DBG_EXCEPTION_NOT_HANDLED
                        stream.write(json.dumps(detail, ensure_ascii=False) + '\n')
                        stream.flush()
                    finally:
                        if not kernel.ContinueDebugEvent(event.pid, event.tid, disposition):
                            raise c.WinError(c.get_last_error())
                    if receipt['processes'] and not live:
                        receipt['lifecycle_reconciled'] = True
                        break
    except BaseException as error:
        receipt['error'] = str(error) or type(error).__name__
    finally:
        if job:
            job.close()
        if process_handle and not assigned:
            _winapi.TerminateProcess(process_handle, 1)
        if thread_handle:
            kernel.CloseHandle(thread_handle)
        if process_handle:
            kernel.CloseHandle(process_handle)
        batch.write(output / 'lifecycle.json', receipt)
    return receipt


if __name__ == '__main__':
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--output', required=True)
    parser.add_argument('--timeout', type=float, default=600)
    parser.add_argument('command', nargs=argparse.REMAINDER)
    args = parser.parse_args()
    command = args.command[1:] if args.command[:1] == ['--'] else args.command
    if not command or not 0 < args.timeout <= 3600:
        parser.error('Supply a command and a timeout in (0, 3600]')
    result = observe(command, args.output, args.timeout)
    print(json.dumps({key: result[key] for key in
                     ('lifecycle_reconciled', 'workload_succeeded', 'workload_exit_code', 'error')}))
    raise SystemExit(0 if result['lifecycle_reconciled'] and result['workload_succeeded'] else 1)
