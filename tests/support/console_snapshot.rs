//! Read-only native console evidence shared by resize fixtures.
use anyhow::{Context, Result, ensure};
use blueberry::probe::Harness;
use std::{
    fs,
    os::windows::{io::FromRawHandle, process::CommandExt},
    path::Path,
};
use windows_sys::Win32::{
    Foundation::{GENERIC_READ, INVALID_HANDLE_VALUE},
    Storage::FileSystem::{CreateFileW, FILE_SHARE_READ, FILE_SHARE_WRITE, OPEN_EXISTING},
    System::Console::{
        AttachConsole, CONSOLE_SCREEN_BUFFER_INFO, COORD, FreeConsole, GetConsoleScreenBufferInfo,
        ReadConsoleOutputCharacterW,
    },
};

pub fn write_attached() -> Result<()> {
    let pid = std::env::var("BLUEBERRY_SNAPSHOT_PID")?.parse()?;
    let path = std::env::var("BLUEBERRY_SNAPSHOT_PATH")?;
    unsafe { FreeConsole() };
    ensure!(
        unsafe { AttachConsole(pid) } != 0,
        "attach snapshot console"
    );
    let name: Vec<u16> = "CONOUT$".encode_utf16().chain(Some(0)).collect();
    let output = unsafe {
        CreateFileW(
            name.as_ptr(),
            GENERIC_READ,
            FILE_SHARE_READ | FILE_SHARE_WRITE,
            std::ptr::null(),
            OPEN_EXISTING,
            0,
            std::ptr::null_mut(),
        )
    };
    ensure!(
        !output.is_null() && output != INVALID_HANDLE_VALUE,
        "open snapshot console"
    );
    let _handle = unsafe { std::os::windows::io::OwnedHandle::from_raw_handle(output) };
    let mut info: CONSOLE_SCREEN_BUFFER_INFO = unsafe { std::mem::zeroed() };
    ensure!(
        unsafe { GetConsoleScreenBufferInfo(output, &mut info) } != 0,
        "read console size"
    );
    let mut lines = Vec::new();
    for y in info.srWindow.Top..=info.srWindow.Bottom {
        let mut row = vec![0u16; (info.srWindow.Right - info.srWindow.Left + 1) as usize];
        let mut read = 0;
        ensure!(
            unsafe {
                ReadConsoleOutputCharacterW(
                    output,
                    row.as_mut_ptr(),
                    row.len() as u32,
                    COORD {
                        X: info.srWindow.Left,
                        Y: y,
                    },
                    &mut read,
                )
            } != 0,
            "read console row"
        );
        lines.push(String::from_utf16_lossy(&row[..read as usize]));
    }
    fs::write(
        path,
        serde_json::to_vec_pretty(&serde_json::json!({
            "width":info.dwSize.X, "height":info.dwSize.Y,
            "cursor":[info.dwCursorPosition.X,info.dwCursorPosition.Y],
            "screen":lines.join("\n")
        }))?,
    )?;
    Ok(())
}

pub fn capture(harness: &Harness, path: &Path) -> Result<()> {
    let result = std::process::Command::new(std::env::current_exe()?)
        .args(["console_snapshot_helper", "--exact", "--ignored"])
        .env(
            "BLUEBERRY_SNAPSHOT_PID",
            harness
                .process_id()
                .context("missing host PID")?
                .to_string(),
        )
        .env("BLUEBERRY_SNAPSHOT_PATH", path)
        .creation_flags(windows_sys::Win32::System::Threading::CREATE_NO_WINDOW)
        .output()?;
    ensure!(
        result.status.success(),
        "console snapshot failed: {} {}",
        String::from_utf8_lossy(&result.stdout),
        String::from_utf8_lossy(&result.stderr)
    );
    Ok(())
}
