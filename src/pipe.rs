//! Session-private named-pipe transport used by the explicit A/B experiment.
//!
//! The OSC protocol remains the default transport.  This module deliberately
//! exposes a small, synchronous API so the host can keep its existing event
//! loop and ordering barriers: a read or write either completes immediately
//! or reports `WouldBlock`.  There is no polling thread and no background
//! lifetime that could outlive a Blueberry session.

use serde_json::Value;
use std::io;

#[cfg(windows)]
mod windows_pipe {
    use super::*;
    use std::{
        ffi::c_void,
        io::{Error, ErrorKind},
        mem,
        os::windows::io::{AsRawHandle, FromRawHandle, OwnedHandle, RawHandle},
        ptr,
    };

    const MAX_FRAME_BYTES: usize = 1024 * 1024;
    const PIPE_ACCESS_DUPLEX: u32 = 0x0000_0003;
    const FILE_FLAG_FIRST_PIPE_INSTANCE: u32 = 0x0008_0000;
    const PIPE_TYPE_MESSAGE: u32 = 0x0000_0004;
    const PIPE_READMODE_MESSAGE: u32 = 0x0000_0002;
    const PIPE_NOWAIT: u32 = 0x0000_0001;
    const PIPE_REJECT_REMOTE_CLIENTS: u32 = 0x0000_0008;
    const ERROR_INSUFFICIENT_BUFFER: u32 = 122;
    const ERROR_PIPE_CONNECTED: u32 = 535;
    const ERROR_PIPE_LISTENING: u32 = 536;
    const ERROR_NO_DATA: u32 = 232;
    const ERROR_PIPE_NOT_CONNECTED: u32 = 233;
    const ERROR_BROKEN_PIPE: u32 = 109;
    const ERROR_MORE_DATA: u32 = 234;
    const TOKEN_QUERY: u32 = 0x0008;
    const TOKEN_USER: u32 = 1;
    const SDDL_REVISION_1: u32 = 1;
    const INVALID_HANDLE_VALUE: RawHandle = -1isize as RawHandle;

    type Bool = i32;

    #[repr(C)]
    struct SecurityAttributes {
        length: u32,
        descriptor: *mut c_void,
        inherit_handle: Bool,
    }

    #[repr(C)]
    struct SidAndAttributes {
        sid: *mut c_void,
        attributes: u32,
    }

    #[repr(C)]
    struct TokenUser {
        user: SidAndAttributes,
    }

    // Keep the FFI local to this module.  Using these declarations instead of
    // enabling more windows-sys feature families keeps the pipe prototype
    // independent of the release transport and Cargo feature set.
    #[link(name = "kernel32")]
    unsafe extern "system" {
        fn GetCurrentProcess() -> RawHandle;
        fn GetLastError() -> u32;
        fn CloseHandle(handle: RawHandle) -> Bool;
        fn LocalFree(memory: *mut c_void) -> *mut c_void;
        fn CreateNamedPipeW(
            name: *const u16,
            open_mode: u32,
            pipe_mode: u32,
            max_instances: u32,
            out_buffer_size: u32,
            in_buffer_size: u32,
            default_timeout: u32,
            security_attributes: *const SecurityAttributes,
        ) -> RawHandle;
        fn ConnectNamedPipe(handle: RawHandle, overlapped: *mut c_void) -> Bool;
        fn DisconnectNamedPipe(handle: RawHandle) -> Bool;
        fn GetNamedPipeClientProcessId(handle: RawHandle, process_id: *mut u32) -> Bool;
        fn PeekNamedPipe(
            handle: RawHandle,
            buffer: *mut c_void,
            buffer_size: u32,
            bytes_read: *mut u32,
            total_bytes_available: *mut u32,
            bytes_left_this_message: *mut u32,
        ) -> Bool;
        fn ReadFile(
            handle: RawHandle,
            buffer: *mut c_void,
            bytes_to_read: u32,
            bytes_read: *mut u32,
            overlapped: *mut c_void,
        ) -> Bool;
        fn WriteFile(
            handle: RawHandle,
            buffer: *const c_void,
            bytes_to_write: u32,
            bytes_written: *mut u32,
            overlapped: *mut c_void,
        ) -> Bool;
    }

    #[link(name = "advapi32")]
    unsafe extern "system" {
        fn OpenProcessToken(process: RawHandle, desired_access: u32, token: *mut RawHandle)
        -> Bool;
        fn GetTokenInformation(
            token: RawHandle,
            information_class: u32,
            information: *mut c_void,
            information_length: u32,
            return_length: *mut u32,
        ) -> Bool;
        fn ConvertSidToStringSidW(sid: *const c_void, string_sid: *mut *mut u16) -> Bool;
        fn ConvertStringSecurityDescriptorToSecurityDescriptorW(
            string_security_descriptor: *const u16,
            string_sd_revision: u32,
            security_descriptor: *mut *mut c_void,
            security_descriptor_size: *mut u32,
        ) -> Bool;
    }

    fn last_error() -> io::Error {
        Error::from_raw_os_error(unsafe { GetLastError() as i32 })
    }

    fn error_with_code(code: u32) -> io::Error {
        Error::from_raw_os_error(code as i32)
    }

    fn wide_z(value: &str) -> Vec<u16> {
        value.encode_utf16().chain(std::iter::once(0)).collect()
    }

    fn sid_string_for_current_process() -> io::Result<String> {
        let mut token: RawHandle = ptr::null_mut();
        let opened = unsafe { OpenProcessToken(GetCurrentProcess(), TOKEN_QUERY, &mut token) };
        if opened == 0 {
            return Err(last_error());
        }

        let result = (|| {
            let mut size = 0u32;
            let first =
                unsafe { GetTokenInformation(token, TOKEN_USER, ptr::null_mut(), 0, &mut size) };
            if first != 0 || size == 0 {
                return Err(last_error());
            }
            let code = unsafe { GetLastError() };
            if code != ERROR_INSUFFICIENT_BUFFER {
                return Err(error_with_code(code));
            }

            let mut bytes = vec![0u8; size as usize];
            let ok = unsafe {
                GetTokenInformation(
                    token,
                    TOKEN_USER,
                    bytes.as_mut_ptr().cast(),
                    bytes.len() as u32,
                    &mut size,
                )
            };
            if ok == 0 {
                return Err(last_error());
            }
            if bytes.len() < mem::size_of::<TokenUser>() {
                return Err(Error::new(
                    ErrorKind::InvalidData,
                    "GetTokenInformation returned a short TOKEN_USER",
                ));
            }
            let user = unsafe { &*(bytes.as_ptr().cast::<TokenUser>()) };
            if user.user.sid.is_null() {
                return Err(Error::new(
                    ErrorKind::InvalidData,
                    "current token has no SID",
                ));
            }

            let mut string_sid = ptr::null_mut();
            let ok = unsafe { ConvertSidToStringSidW(user.user.sid, &mut string_sid) };
            if ok == 0 || string_sid.is_null() {
                return Err(last_error());
            }
            let sid = unsafe {
                let mut length = 0usize;
                while *string_sid.add(length) != 0 {
                    length += 1;
                }
                String::from_utf16(std::slice::from_raw_parts(string_sid, length))
                    .map_err(|_| Error::new(ErrorKind::InvalidData, "current SID is not UTF-16"))
            };
            unsafe {
                LocalFree(string_sid.cast());
            }
            sid
        })();

        unsafe {
            CloseHandle(token);
        }
        result
    }

    struct SecurityDescriptor {
        descriptor: *mut c_void,
        attributes: SecurityAttributes,
    }

    impl SecurityDescriptor {
        fn for_current_user() -> io::Result<Self> {
            let sid = sid_string_for_current_process()?;
            // SYSTEM and the current token SID get full access.  The named
            // pipe itself rejects remote clients, so this ACL is the final
            // process-level boundary for the local session transport.
            let sddl = format!("D:(A;;GA;;;SY)(A;;GA;;;{sid})");
            let wide = wide_z(&sddl);
            let mut descriptor = ptr::null_mut();
            let ok = unsafe {
                ConvertStringSecurityDescriptorToSecurityDescriptorW(
                    wide.as_ptr(),
                    SDDL_REVISION_1,
                    &mut descriptor,
                    ptr::null_mut(),
                )
            };
            if ok == 0 || descriptor.is_null() {
                return Err(last_error());
            }
            Ok(Self {
                descriptor,
                attributes: SecurityAttributes {
                    length: mem::size_of::<SecurityAttributes>() as u32,
                    descriptor,
                    inherit_handle: 0,
                },
            })
        }
    }

    impl Drop for SecurityDescriptor {
        fn drop(&mut self) {
            unsafe {
                LocalFree(self.descriptor);
            }
        }
    }

    /// A single-instance, local, message-mode duplex named pipe.
    pub struct PipeServer {
        name: String,
        handle: Option<OwnedHandle>,
        expected_client_pid: Option<u32>,
        connected: bool,
    }

    impl PipeServer {
        pub fn new() -> io::Result<Self> {
            let name = format!("blueberry-{}", uuid::Uuid::new_v4().simple());
            let full_name = format!(r"\\.\pipe\{name}");
            let name_wide = wide_z(&full_name);
            let security = SecurityDescriptor::for_current_user()?;
            let pipe_mode = PIPE_TYPE_MESSAGE
                | PIPE_READMODE_MESSAGE
                | PIPE_NOWAIT
                | PIPE_REJECT_REMOTE_CLIENTS;
            let handle = unsafe {
                CreateNamedPipeW(
                    name_wide.as_ptr(),
                    PIPE_ACCESS_DUPLEX | FILE_FLAG_FIRST_PIPE_INSTANCE,
                    pipe_mode,
                    1,
                    MAX_FRAME_BYTES as u32,
                    MAX_FRAME_BYTES as u32,
                    0,
                    &security.attributes,
                )
            };
            if handle == INVALID_HANDLE_VALUE || handle.is_null() {
                return Err(last_error());
            }
            // OwnedHandle takes over only after all creation checks pass.
            let handle = unsafe { OwnedHandle::from_raw_handle(handle) };
            Ok(Self {
                name,
                handle: Some(handle),
                expected_client_pid: None,
                connected: false,
            })
        }

        pub fn name(&self) -> &str {
            &self.name
        }

        pub fn set_client_pid(&mut self, pid: u32) {
            self.expected_client_pid = (pid != 0).then_some(pid);
        }

        /// Make the server unusable and close the pipe.  The host uses this
        /// when a single transport frame fails so subsequent events fall back
        /// to OSC/file payloads instead of trying to reuse a desynchronised
        /// stream.
        pub fn disable(&mut self) {
            if let Some(handle) = self.handle.take() {
                if self.connected {
                    unsafe {
                        DisconnectNamedPipe(handle.as_raw_handle());
                    }
                }
                drop(handle);
            }
            self.connected = false;
        }

        fn handle(&self) -> io::Result<RawHandle> {
            self.handle
                .as_ref()
                .map(AsRawHandle::as_raw_handle)
                .ok_or_else(|| Error::new(ErrorKind::NotConnected, "named pipe is disabled"))
        }

        fn client_pid(&self, handle: RawHandle) -> io::Result<Option<u32>> {
            let mut pid = 0u32;
            let ok = unsafe { GetNamedPipeClientProcessId(handle, &mut pid) };
            if ok != 0 {
                return Ok(Some(pid));
            }
            let error = unsafe { GetLastError() };
            if error == ERROR_PIPE_NOT_CONNECTED || error == ERROR_BROKEN_PIPE {
                return Ok(None);
            }
            Err(error_with_code(error))
        }

        fn ensure_connection(&mut self) -> io::Result<RawHandle> {
            let handle = self.handle()?;
            let expected = self.expected_client_pid.ok_or_else(|| {
                Error::new(
                    ErrorKind::InvalidInput,
                    "named-pipe client PID must be set before I/O",
                )
            })?;

            if !self.connected {
                let ok = unsafe { ConnectNamedPipe(handle, ptr::null_mut()) };
                if ok != 0 {
                    self.connected = true;
                } else {
                    match unsafe { GetLastError() } {
                        ERROR_PIPE_CONNECTED => self.connected = true,
                        ERROR_PIPE_LISTENING | ERROR_NO_DATA => {
                            // A nonblocking listener has no client yet.  A
                            // client can connect between this call and the
                            // next event-loop turn.
                            if self.client_pid(handle)?.is_none() {
                                return Err(Error::new(
                                    ErrorKind::WouldBlock,
                                    "named pipe has no connected client",
                                ));
                            }
                            self.connected = true;
                        }
                        _ => return Err(last_error()),
                    }
                }
            }

            match self.client_pid(handle)? {
                Some(pid) if pid == expected => Ok(handle),
                Some(_) => {
                    // Never let a client with a different process identity
                    // keep this session instance alive.
                    unsafe {
                        DisconnectNamedPipe(handle);
                    }
                    self.connected = false;
                    self.disable();
                    Err(Error::new(
                        ErrorKind::PermissionDenied,
                        "named-pipe client PID does not match the Blueberry child",
                    ))
                }
                None => {
                    self.connected = false;
                    Err(Error::new(
                        ErrorKind::NotConnected,
                        "named pipe client disconnected",
                    ))
                }
            }
        }

        pub fn read_json(&mut self) -> io::Result<Value> {
            let handle = self.ensure_connection()?;
            let mut total_available = 0u32;
            let mut bytes_left = 0u32;
            let ok = unsafe {
                PeekNamedPipe(
                    handle,
                    ptr::null_mut(),
                    0,
                    ptr::null_mut(),
                    &mut total_available,
                    &mut bytes_left,
                )
            };
            if ok == 0 {
                let error = unsafe { GetLastError() };
                if matches!(
                    error,
                    ERROR_NO_DATA | ERROR_PIPE_NOT_CONNECTED | ERROR_BROKEN_PIPE
                ) {
                    return Err(Error::new(
                        ErrorKind::WouldBlock,
                        "named pipe has no complete frame",
                    ));
                }
                return Err(error_with_code(error));
            }
            let frame_size = if bytes_left != 0 {
                bytes_left
            } else {
                total_available
            } as usize;
            if frame_size == 0 {
                return Err(Error::new(
                    ErrorKind::WouldBlock,
                    "named pipe has no complete frame",
                ));
            }
            if frame_size > MAX_FRAME_BYTES {
                self.disable();
                return Err(Error::new(
                    ErrorKind::InvalidData,
                    "named-pipe frame exceeds the 1 MiB limit",
                ));
            }

            let mut frame = vec![0u8; frame_size];
            let mut read = 0u32;
            let ok = unsafe {
                ReadFile(
                    handle,
                    frame.as_mut_ptr().cast(),
                    frame.len() as u32,
                    &mut read,
                    ptr::null_mut(),
                )
            };
            if ok == 0 {
                let error = unsafe { GetLastError() };
                return Err(if error == ERROR_MORE_DATA {
                    Error::new(
                        ErrorKind::InvalidData,
                        "named-pipe frame was only partially read",
                    )
                } else {
                    error_with_code(error)
                });
            }
            if read as usize != frame.len() {
                return Err(Error::new(
                    ErrorKind::UnexpectedEof,
                    "named-pipe frame length changed while reading",
                ));
            }
            while frame
                .last()
                .is_some_and(|byte| *byte == b'\n' || *byte == b'\r')
            {
                frame.pop();
            }
            serde_json::from_slice(&frame).map_err(|error| {
                Error::new(
                    ErrorKind::InvalidData,
                    format!("invalid UTF-8/JSON named-pipe frame: {error}"),
                )
            })
        }

        pub fn write_json(&mut self, value: &Value) -> io::Result<()> {
            let mut frame = serde_json::to_vec(value).map_err(|error| {
                Error::new(
                    ErrorKind::InvalidData,
                    format!("could not serialize named-pipe frame: {error}"),
                )
            })?;
            frame.push(b'\n');
            if frame.len() > MAX_FRAME_BYTES {
                return Err(Error::new(
                    ErrorKind::InvalidData,
                    "named-pipe frame exceeds the 1 MiB limit",
                ));
            }
            let handle = self.ensure_connection()?;
            let mut written = 0u32;
            let ok = unsafe {
                WriteFile(
                    handle,
                    frame.as_ptr().cast(),
                    frame.len() as u32,
                    &mut written,
                    ptr::null_mut(),
                )
            };
            if ok == 0 {
                let error = last_error();
                self.disable();
                return Err(error);
            }
            if written as usize != frame.len() {
                self.disable();
                return Err(Error::new(
                    ErrorKind::WriteZero,
                    "named-pipe frame was only partially written",
                ));
            }
            Ok(())
        }
    }

    impl Drop for PipeServer {
        fn drop(&mut self) {
            self.disable();
        }
    }

    #[cfg(test)]
    mod tests {
        use super::*;
        use std::{
            fs::OpenOptions,
            io::{Read, Write},
            thread,
        };

        fn connect(name: &str) -> std::fs::File {
            let path = format!(r"\\.\pipe\{name}");
            for _ in 0..100 {
                match OpenOptions::new().read(true).write(true).open(&path) {
                    Ok(file) => return file,
                    Err(error) if error.kind() == ErrorKind::WouldBlock => {
                        thread::yield_now();
                    }
                    Err(error) => panic!("connect named pipe: {error}"),
                }
            }
            panic!("timed out connecting named pipe")
        }

        #[test]
        fn rejects_a_client_with_the_wrong_process_identity() {
            let mut server = PipeServer::new().expect("create pipe");
            server.set_client_pid(u32::MAX);
            let _client = connect(server.name());
            let error = server.read_json().expect_err("wrong PID must be rejected");
            assert_eq!(error.kind(), ErrorKind::PermissionDenied);
        }

        #[test]
        fn preserves_message_order_and_unicode() {
            let mut server = PipeServer::new().expect("create pipe");
            server.set_client_pid(std::process::id());
            let mut client = connect(server.name());
            let first =
                serde_json::to_vec(&serde_json::json!({"sequence":1,"text":"中文😀"})).unwrap();
            let second =
                serde_json::to_vec(&serde_json::json!({"sequence":2,"text":"第二帧"})).unwrap();
            let mut first_frame = first;
            first_frame.push(b'\n');
            let mut second_frame = second;
            second_frame.push(b'\n');
            // Each JSON line must be emitted by one write call; a separate
            // newline write is a second message on a message-mode pipe.
            client.write_all(&first_frame).unwrap();
            client.write_all(&second_frame).unwrap();
            assert_eq!(server.read_json().unwrap()["sequence"], 1);
            assert_eq!(server.read_json().unwrap()["text"], "第二帧");
            assert_eq!(
                server.read_json().unwrap_err().kind(),
                ErrorKind::WouldBlock
            );
        }

        #[test]
        fn a_disconnected_client_is_reported_without_waiting() {
            let mut server = PipeServer::new().expect("create pipe");
            server.set_client_pid(std::process::id());
            let client = connect(server.name());
            drop(client);
            let error = server
                .read_json()
                .expect_err("closed client must not block");
            assert!(matches!(
                error.kind(),
                ErrorKind::NotConnected | ErrorKind::WouldBlock
            ));
        }

        #[test]
        fn oversized_frame_is_rejected_before_the_write() {
            let mut server = PipeServer::new().expect("create pipe");
            let value = serde_json::json!({"text":"x".repeat(MAX_FRAME_BYTES)});
            let error = server.write_json(&value).expect_err("oversized frame");
            assert_eq!(error.kind(), ErrorKind::InvalidData);
            server.disable();
        }

        #[allow(dead_code)]
        fn read_client_frame(client: &mut std::fs::File) -> String {
            let mut bytes = Vec::new();
            client.read_to_end(&mut bytes).unwrap();
            String::from_utf8(bytes).unwrap()
        }
    }
}

#[cfg(windows)]
pub use windows_pipe::PipeServer;

#[cfg(not(windows))]
/// Non-Windows builds keep the public API available so the default OSC host
/// and cross-platform unit tests compile.  The explicit pipe experiment is a
/// Windows-only transport.
pub struct PipeServer;

#[cfg(not(windows))]
impl PipeServer {
    pub fn new() -> io::Result<Self> {
        Err(io::Error::new(
            io::ErrorKind::Unsupported,
            "Blueberry named-pipe transport is Windows-only",
        ))
    }

    pub fn name(&self) -> &str {
        ""
    }

    pub fn set_client_pid(&mut self, _pid: u32) {}

    pub fn read_json(&mut self) -> io::Result<Value> {
        Err(io::Error::new(
            io::ErrorKind::Unsupported,
            "Blueberry named-pipe transport is Windows-only",
        ))
    }

    pub fn write_json(&mut self, _value: &Value) -> io::Result<()> {
        Err(io::Error::new(
            io::ErrorKind::Unsupported,
            "Blueberry named-pipe transport is Windows-only",
        ))
    }

    pub fn disable(&mut self) {}
}
