"""Run frozen benchmark tasks sequentially, with owned process-tree cleanup.

Each task supplies argv, cwd, timeout_seconds, and a JSON report whose
completion_field must be true. Exit 1 with a complete report is a measured
gate failure; a missing/incomplete report is a collection failure. --inspect
reconciles a receipt left running by an abruptly terminated coordinator.
"""
import argparse
import ctypes
import hashlib
import json
import math
import os
from pathlib import Path
import signal
import subprocess
import sys
import time
from datetime import datetime, timezone


def now():
    return datetime.now(timezone.utc).isoformat()


def read(path):
    return json.loads(Path(path).read_text(encoding="utf-8-sig"))


def write(path, value):
    path = Path(path)
    temporary = path.with_name(path.name + ".tmp")
    temporary.write_text(json.dumps(value, ensure_ascii=False, indent=2) + "\n", encoding="utf-8")
    os.replace(temporary, path)


def sha(path):
    with Path(path).open("rb") as stream:
        return hashlib.file_digest(stream, "sha256").hexdigest().upper()


def process_identity(pid):
    """PID plus creation time, so inspection never trusts a recycled PID."""
    if os.name == "nt":
        from ctypes import wintypes as w
        kernel = ctypes.WinDLL("kernel32", use_last_error=True)
        kernel.OpenProcess.argtypes = [w.DWORD, w.BOOL, w.DWORD]
        kernel.OpenProcess.restype = w.HANDLE
        kernel.GetProcessTimes.argtypes = [w.HANDLE] + [ctypes.POINTER(w.FILETIME)] * 4
        kernel.GetExitCodeProcess.argtypes = [w.HANDLE, ctypes.POINTER(w.DWORD)]
        kernel.CloseHandle.argtypes = [w.HANDLE]
        handle = kernel.OpenProcess(0x1000, False, pid)
        if not handle:
            return None
        try:
            code = w.DWORD()
            if not kernel.GetExitCodeProcess(handle, ctypes.byref(code)) or code.value != 259:
                return None
            values = [w.FILETIME() for _ in range(4)]
            if not kernel.GetProcessTimes(handle, *(ctypes.byref(v) for v in values)):
                return None
            created = values[0].dwLowDateTime + (values[0].dwHighDateTime << 32)
            return {"pid": pid, "created": created}
        finally:
            kernel.CloseHandle(handle)
    try:
        # Linux task start ticks are independent of PID reuse.
        fields = Path(f"/proc/{pid}/stat").read_text().rsplit(")", 1)[1].split()
        if fields[0] == "Z":
            return None
        return {"pid": pid, "created": fields[19]}
    except (FileNotFoundError, PermissionError, IndexError):
        return None


class WindowsJob:
    def __init__(self):
        from ctypes import wintypes as w
        class Limits(ctypes.Structure):
            _fields_ = [("process_time", ctypes.c_int64), ("job_time", ctypes.c_int64),
                        ("flags", w.DWORD), ("minimum", ctypes.c_size_t), ("maximum", ctypes.c_size_t),
                        ("active", w.DWORD), ("affinity", ctypes.c_size_t),
                        ("priority", w.DWORD), ("scheduling", w.DWORD)]
        class Extended(ctypes.Structure):
            _fields_ = [("limits", Limits), ("io", ctypes.c_uint64 * 6),
                        ("process_memory", ctypes.c_size_t), ("job_memory", ctypes.c_size_t),
                        ("peak_process", ctypes.c_size_t), ("peak_job", ctypes.c_size_t)]
        self.kernel = ctypes.WinDLL("kernel32", use_last_error=True)
        self.kernel.CreateJobObjectW.argtypes = [ctypes.c_void_p, w.LPCWSTR]
        self.kernel.CreateJobObjectW.restype = w.HANDLE
        self.kernel.SetInformationJobObject.argtypes = [w.HANDLE, ctypes.c_int, ctypes.c_void_p, w.DWORD]
        self.kernel.AssignProcessToJobObject.argtypes = [w.HANDLE, w.HANDLE]
        self.kernel.CloseHandle.argtypes = [w.HANDLE]
        self.handle = self.kernel.CreateJobObjectW(None, None)
        if not self.handle:
            raise ctypes.WinError(ctypes.get_last_error())
        limits = Extended()
        limits.limits.flags = 0x2000  # JOB_OBJECT_LIMIT_KILL_ON_JOB_CLOSE
        if not self.kernel.SetInformationJobObject(self.handle, 9, ctypes.byref(limits), ctypes.sizeof(limits)):
            error = ctypes.WinError(ctypes.get_last_error())
            self.close()
            raise error

    def assign(self, process):
        if not self.kernel.AssignProcessToJobObject(self.handle, int(process._handle)):
            raise ctypes.WinError(ctypes.get_last_error())

    def close(self):
        if self.handle:
            self.kernel.CloseHandle(self.handle)
            self.handle = None


def validate(plan):
    assert plan.get("schema") == 1, "Unsupported benchmark plan"
    assert plan.get("tasks"), "Empty benchmark plan"
    names = set()
    for task in plan["tasks"]:
        name = task["name"]
        assert name not in names and name and all(c.isalnum() or c in "-_" for c in name), "Invalid task name"
        names.add(name)
        assert isinstance(task["argv"], list) and task["argv"] and all(isinstance(x, str) for x in task["argv"]), "Invalid argv"
        timeout = task["timeout_seconds"]
        assert type(timeout) in (int, float) and math.isfinite(timeout) and timeout > 0, "Invalid timeout"
        assert Path(task["cwd"]).is_dir(), "Missing task working directory"
        assert not Path(task["report"]).exists(), "Refusing existing benchmark report"
    for dependency in plan.get("dependencies", []):
        assert sha(dependency["path"]) == dependency["sha256"].upper(), "Frozen dependency identity mismatch"


def inspect(folder):
    path = Path(folder) / "batch.json"
    receipt = read(path)
    if receipt["status"] in ("running", "interrupted"):
        owner = receipt["owner"]
        if process_identity(owner["pid"]) != owner:
            if receipt["status"] == "running":
                receipt.update(status="interrupted", measurement_complete=False, interrupted_at_utc=now(),
                               interruption="Coordinator identity is no longer alive; retained reports are not a complete batch.")
            for entry in receipt.get("results", []):
                if entry.get("status") == "running":
                    worker = entry.get("worker")
                    entry.update(status="interrupted", stopped_at_utc=receipt["interrupted_at_utc"],
                                 worker_alive=bool(worker and process_identity(worker["pid"]) == worker))
            write(path, receipt)
    return receipt


def run(plan_path, folder):
    plan = read(plan_path)
    validate(plan)
    folder = Path(folder).resolve()
    folder.mkdir(parents=True, exist_ok=False)
    frozen = folder / "plan.json"
    write(frozen, plan)
    receipt = dict(schema=1, status="running", started_at_utc=now(), owner=process_identity(os.getpid()),
                   plan_sha256=sha(frozen), runner_sha256=sha(__file__), measurement_complete=False, results=[])
    path = folder / "batch.json"
    write(path, receipt)
    try:
        for task in plan["tasks"]:
            job = WindowsJob() if os.name == "nt" else None
            process = None
            entry = dict(name=task["name"], started_at_utc=now(), status="running", report=task["report"])
            receipt["results"].append(entry)
            spec = folder / (task["name"] + "-worker.json")
            write(spec, task)
            try:
                with (folder / (task["name"] + ".log")).open("wb") as log:
                    process = subprocess.Popen([sys.executable, str(Path(__file__).resolve()), "--worker", str(spec)],
                                               stdin=subprocess.PIPE, stdout=log, stderr=subprocess.STDOUT,
                                               # A hidden worker console must not inherit the
                                               # caller's console. Do not disable Ctrl+C with
                                               # CREATE_NEW_PROCESS_GROUP: Shell descendants
                                               # must retain their normal interrupt behavior.
                                               creationflags=0x08000000 if os.name == "nt" else 0,
                                               start_new_session=os.name != "nt")
                    # The helper waits on stdin. Assign it before it can spawn
                    # the real worker or any probe, avoiding an ownership race.
                    if job:
                        job.assign(process)
                    entry["worker"] = process_identity(process.pid)
                    write(path, receipt)
                    process.stdin.write(b"G")
                    process.stdin.close()
                    entry["exit_code"] = process.wait(timeout=task["timeout_seconds"])
                report = read(task["report"])
                assert report.get(task["completion_field"]) is True, "Worker report is incomplete"
                assert entry["exit_code"] in task.get("accepted_exit_codes", [0]), "Unexpected worker exit code"
                entry.update(status="completed", report_sha256=sha(task["report"]), completed_at_utc=now())
            except BaseException as error:
                entry.update(status="invalid", error=str(error) or type(error).__name__, stopped_at_utc=now())
                raise
            finally:
                if job:
                    job.close()
                elif process:
                    try:
                        os.killpg(process.pid, signal.SIGKILL)
                    except ProcessLookupError:
                        pass
                if process:
                    if process.poll() is None:
                        process.kill()
                    process.wait(timeout=10)
                write(path, receipt)
        for dependency in plan.get("dependencies", []):
            assert sha(dependency["path"]) == dependency["sha256"].upper(), "Dependency changed during measurement"
        assert sha(__file__) == receipt["runner_sha256"], "Runner changed during measurement"
        receipt.update(status="completed", measurement_complete=True, completed_at_utc=now())
    except BaseException as error:
        receipt.update(status="invalid", error=str(error) or type(error).__name__, stopped_at_utc=now())
        raise
    finally:
        write(path, receipt)
    return receipt


def detach(plan_path, folder):
    """Do not report spawn success as measurement success, or silently fall
    back into a caller-owned Job which would lose a long run on interruption.
    The detached coordinator still owns/kills every measurement worker Job.
    """
    if os.name != "nt":
        raise OSError("Detached benchmark launch is Windows-only")
    plan_path, folder = Path(plan_path).resolve(strict=True), Path(folder).resolve()
    validate(read(plan_path))
    assert not folder.exists(), "Refusing existing benchmark directory"
    receipt_path = folder.with_name(folder.name + ".launch.json")
    log_path = folder.with_name(folder.name + ".launch.log")
    receipt = dict(schema=1, status="preparing", measurement_complete=False,
                   plan=str(plan_path), plan_sha256=sha(plan_path),
                   report=str(folder / "batch.json"), started_at_utc=now())
    # Exclusive reservation also prevents two launchers racing on one output.
    with receipt_path.open("x", encoding="utf-8") as stream:
        json.dump(receipt, stream)
    try:
        with log_path.open("xb") as log:
            process = subprocess.Popen([sys.executable, str(Path(__file__).resolve()),
                "--plan", str(plan_path), "--output", str(folder)],
                stdin=subprocess.DEVNULL, stdout=log, stderr=subprocess.STDOUT,
                # DETACHED_PROCESS keeps the launch hidden. Breakaway is an
                # explicit Windows permission check; no fallback bypasses it.
                creationflags=0x00000008 | 0x01000000, close_fds=True)
        receipt.update(status="started", owner=process_identity(process.pid))
        assert receipt["owner"] is not None, "Coordinator exited during launch; inspect launch log"
    except BaseException as error:
        receipt.update(status="failed", error=str(error) or type(error).__name__)
        raise
    finally:
        write(receipt_path, receipt)
    print(json.dumps(receipt, ensure_ascii=False))
    return receipt


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--plan", type=Path)
    parser.add_argument("--output", type=Path)
    parser.add_argument("--inspect", type=Path)
    parser.add_argument("--detach", action="store_true", help="Windows: start an owned coordinator outside the caller's Job; fail if breakaway is denied")
    parser.add_argument("--worker", type=Path, help=argparse.SUPPRESS)
    args = parser.parse_args()
    if args.worker:
        if sys.stdin.buffer.read(1) != b"G":
            return 2
        task = read(args.worker)
        environment = dict(os.environ, **task.get("environment", {}))
        return subprocess.call(task["argv"], cwd=task["cwd"], env=environment, stdin=subprocess.DEVNULL)
    if args.inspect:
        print(json.dumps(inspect(args.inspect), ensure_ascii=False))
        return 0
    if not (args.plan and args.output):
        parser.error("--plan and --output required")
    if args.detach:
        detach(args.plan, args.output)
    else:
        run(args.plan, args.output)
    return 0


if __name__ == "__main__":
    sys.exit(main())
