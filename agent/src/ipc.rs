//! The agent's local socket, which the native-messaging host connects to: a
//! Unix socket in the user's own cache folder (mode 0700) on macOS, and on
//! Windows a named pipe that only the signed-in user (and SYSTEM) can open,
//! created with `FILE_FLAG_FIRST_PIPE_INSTANCE` so another program can't
//! claim the name first.
//!
//! Frames are what the browser uses on the host's stdin and stdout: a 4-byte
//! native-endian length, then that many bytes of JSON, 1 MB at most (as
//! upstream's `desktop_native/core/src/ipc`).

use std::{
    io::{self, Read, Write},
    path::PathBuf,
};

/// The largest frame either side accepts, as the browser's native messaging.
pub const MAX_FRAME: usize = 1024 * 1024;

/// Reads one frame. `Ok(None)` at a clean end of stream.
pub fn read_frame(reader: &mut impl Read) -> io::Result<Option<Vec<u8>>> {
    let mut length = [0u8; 4];
    match reader.read_exact(&mut length) {
        Ok(()) => {}
        Err(e) if e.kind() == io::ErrorKind::UnexpectedEof => return Ok(None),
        Err(e) => return Err(e),
    }
    let length = u32::from_ne_bytes(length) as usize;
    if length > MAX_FRAME {
        return Err(io::Error::new(
            io::ErrorKind::InvalidData,
            format!("a {length}-byte frame is over the limit"),
        ));
    }
    let mut frame = vec![0u8; length];
    reader.read_exact(&mut frame)?;
    Ok(Some(frame))
}

pub fn write_frame(writer: &mut impl Write, frame: &[u8]) -> io::Result<()> {
    if frame.len() > MAX_FRAME {
        return Err(io::Error::new(
            io::ErrorKind::InvalidInput,
            "frame over the limit",
        ));
    }
    writer.write_all(&(frame.len() as u32).to_ne_bytes())?;
    writer.write_all(frame)?;
    writer.flush()
}

/// Where the agent listens, per user: a hash of the home folder, as upstream
/// names its pipe, so two people on one computer never share one.
pub fn address() -> PathBuf {
    // End-to-end tests give the agent, and the host the browser starts, a
    // private address. Test builds only.
    #[cfg(feature = "test-authenticator")]
    if let Some(home) = std::env::var_os("PASSBUNDLE_AGENT_TEST_HOME") {
        return address_for(&home);
    }
    let home =
        std::env::var_os(if cfg!(windows) { "USERPROFILE" } else { "HOME" }).unwrap_or_default();
    address_for(&home)
}

fn address_for(home: &std::ffi::OsStr) -> PathBuf {
    #[cfg(windows)]
    {
        use base64::{engine::general_purpose::URL_SAFE_NO_PAD, Engine};
        use sha2::{Digest, Sha256};
        let hash = URL_SAFE_NO_PAD.encode(Sha256::digest(home.as_encoded_bytes()));
        PathBuf::from(format!(
            r"\\.\pipe\{}-agent.{hash}",
            crate::brand::NAME.to_lowercase()
        ))
    }
    #[cfg(not(windows))]
    {
        let base = PathBuf::from(home);
        #[cfg(target_os = "macos")]
        let dir = base
            .join("Library/Caches")
            .join(crate::config::reverse_domain(crate::brand::DOMAIN) + ".agent");
        #[cfg(not(target_os = "macos"))]
        let dir = base
            .join(".cache")
            .join(crate::config::reverse_domain(crate::brand::DOMAIN) + ".agent");
        dir.join("agent.sock")
    }
}

#[cfg(unix)]
pub use unix::{connect, ClientStream, Listener, ServerStream};
#[cfg(windows)]
pub use windows::{connect, ClientStream, Listener, ServerStream};

/// The agent this host was installed with: `passbundle-agent`, next to the
/// host's own program.
pub fn installed_agent() -> io::Result<PathBuf> {
    let host = std::env::current_exe()?;
    Ok(host.with_file_name(format!("passbundle-agent{}", std::env::consts::EXE_SUFFIX)))
}

/// Whether the program at the other end of `stream` is the agent installed
/// next to this host. A program running as the same person could make the
/// pipe or socket first while the agent isn't running, and would then be
/// handed the user key when someone turns on unlock with Windows Hello or
/// Touch ID; so the host asks the operating system who it reached.
pub fn reached_installed_agent(stream: &ClientStream) -> bool {
    let (Ok(expected), Some(peer)) = (installed_agent(), peer_program(stream)) else {
        return false;
    };
    match (std::fs::canonicalize(expected), std::fs::canonicalize(peer)) {
        (Ok(expected), Ok(peer)) => expected == peer,
        _ => false,
    }
}

#[cfg(unix)]
use unix::peer_program;
#[cfg(windows)]
use windows::peer_program;

#[cfg(unix)]
mod unix {
    use std::{
        fs, io,
        os::unix::{
            fs::{DirBuilderExt, PermissionsExt},
            net::{UnixListener, UnixStream},
        },
        path::Path,
    };

    pub type ServerStream = UnixStream;
    pub type ClientStream = UnixStream;

    pub struct Listener(UnixListener);

    impl Listener {
        /// Binds the agent's socket. Only one agent per user runs (see
        /// `instance`), so a socket file already there is a stale one.
        pub fn bind(address: &Path) -> io::Result<Self> {
            let dir = address.parent().expect("socket in a folder");
            fs::DirBuilder::new()
                .recursive(true)
                .mode(0o700)
                .create(dir)?;
            fs::set_permissions(dir, fs::Permissions::from_mode(0o700))?;
            let _ = fs::remove_file(address);
            Ok(Self(UnixListener::bind(address)?))
        }

        pub fn accept(&self) -> io::Result<ServerStream> {
            Ok(self.0.accept()?.0)
        }
    }

    pub fn connect(address: &Path) -> io::Result<ClientStream> {
        UnixStream::connect(address)
    }

    /// The program at the other end of a socket: its process, from the
    /// socket, and that process's executable.
    #[cfg(target_os = "macos")]
    pub fn peer_program(stream: &UnixStream) -> Option<std::path::PathBuf> {
        use std::os::{fd::AsRawFd, unix::ffi::OsStringExt};
        let mut pid: libc::pid_t = 0;
        let mut length = std::mem::size_of::<libc::pid_t>() as libc::socklen_t;
        let ok = unsafe {
            libc::getsockopt(
                stream.as_raw_fd(),
                libc::SOL_LOCAL,
                libc::LOCAL_PEERPID,
                (&mut pid as *mut libc::pid_t).cast(),
                &mut length,
            )
        };
        if ok != 0 || pid <= 0 {
            return None;
        }
        let mut path = vec![0u8; libc::PROC_PIDPATHINFO_MAXSIZE as usize];
        let written =
            unsafe { libc::proc_pidpath(pid, path.as_mut_ptr().cast(), path.len() as u32) };
        if written <= 0 {
            return None;
        }
        path.truncate(written as usize);
        Some(std::ffi::OsString::from_vec(path).into())
    }

    #[cfg(not(target_os = "macos"))]
    pub fn peer_program(stream: &UnixStream) -> Option<std::path::PathBuf> {
        use std::os::fd::AsRawFd;
        let mut credentials: libc::ucred = unsafe { std::mem::zeroed() };
        let mut length = std::mem::size_of::<libc::ucred>() as libc::socklen_t;
        let ok = unsafe {
            libc::getsockopt(
                stream.as_raw_fd(),
                libc::SOL_SOCKET,
                libc::SO_PEERCRED,
                (&mut credentials as *mut libc::ucred).cast(),
                &mut length,
            )
        };
        if ok != 0 || credentials.pid <= 0 {
            return None;
        }
        fs::read_link(format!("/proc/{}/exe", credentials.pid)).ok()
    }
}

#[cfg(windows)]
mod windows {
    use std::{
        ffi::OsStr,
        fs::File,
        io,
        os::windows::{ffi::OsStrExt, io::FromRawHandle},
        path::Path,
        ptr,
        sync::Mutex,
    };

    use windows_sys::Win32::{
        Foundation::{
            CloseHandle, DuplicateHandle, GetLastError, LocalFree, DUPLICATE_SAME_ACCESS,
            ERROR_BROKEN_PIPE, ERROR_IO_PENDING, ERROR_PIPE_BUSY, ERROR_PIPE_CONNECTED,
            GENERIC_READ, GENERIC_WRITE, HANDLE, INVALID_HANDLE_VALUE,
        },
        Security::{
            Authorization::{
                ConvertSidToStringSidW, ConvertStringSecurityDescriptorToSecurityDescriptorW,
                SDDL_REVISION_1,
            },
            GetTokenInformation, TokenUser, PSECURITY_DESCRIPTOR, SECURITY_ATTRIBUTES, TOKEN_QUERY,
            TOKEN_USER,
        },
        Storage::FileSystem::{
            CreateFileW, ReadFile, WriteFile, FILE_FLAG_FIRST_PIPE_INSTANCE, FILE_FLAG_OVERLAPPED,
            OPEN_EXISTING, PIPE_ACCESS_DUPLEX,
        },
        System::{
            Pipes::{
                ConnectNamedPipe, CreateNamedPipeW, GetNamedPipeServerProcessId, WaitNamedPipeW,
                PIPE_READMODE_BYTE, PIPE_REJECT_REMOTE_CLIENTS, PIPE_TYPE_BYTE,
                PIPE_UNLIMITED_INSTANCES, PIPE_WAIT,
            },
            Threading::{CreateEventW, GetCurrentProcess, OpenProcessToken},
            IO::{GetOverlappedResult, OVERLAPPED},
        },
    };

    use super::MAX_FRAME;

    /// The agent's end: one thread per connection reads a request and writes
    /// its reply, one at a time, so ordinary blocking I/O is right.
    pub type ServerStream = File;

    pub struct Listener {
        name: Vec<u16>,
        security: SecurityDescriptor,
        /// A pipe instance waiting for the next client. One always exists
        /// while the agent runs: a pipe name only exists while an instance
        /// does, so without it a client arriving between connections would
        /// find nothing.
        next: Mutex<HANDLE>,
    }

    // The handles and the descriptor are only used under the lock or read.
    unsafe impl Send for Listener {}
    unsafe impl Sync for Listener {}

    fn wide(s: &OsStr) -> Vec<u16> {
        s.encode_wide().chain(Some(0)).collect()
    }

    fn create_instance(
        name: &[u16],
        security: &SecurityDescriptor,
        first: bool,
    ) -> io::Result<HANDLE> {
        let attributes = SECURITY_ATTRIBUTES {
            nLength: std::mem::size_of::<SECURITY_ATTRIBUTES>() as u32,
            lpSecurityDescriptor: security.0,
            bInheritHandle: 0,
        };
        // The first instance must be the first pipe of this name on the
        // computer: if another program made it, refuse to serve on it.
        let first = if first {
            FILE_FLAG_FIRST_PIPE_INSTANCE
        } else {
            0
        };
        let handle = unsafe {
            CreateNamedPipeW(
                name.as_ptr(),
                PIPE_ACCESS_DUPLEX | first,
                PIPE_TYPE_BYTE | PIPE_READMODE_BYTE | PIPE_WAIT | PIPE_REJECT_REMOTE_CLIENTS,
                PIPE_UNLIMITED_INSTANCES,
                (MAX_FRAME + 4) as u32,
                (MAX_FRAME + 4) as u32,
                0,
                &attributes,
            )
        };
        if handle == INVALID_HANDLE_VALUE {
            return Err(io::Error::last_os_error());
        }
        Ok(handle)
    }

    impl Listener {
        pub fn bind(address: &Path) -> io::Result<Self> {
            let name = wide(address.as_os_str());
            let security = SecurityDescriptor::current_user_only()?;
            let first = create_instance(&name, &security, true)?;
            Ok(Self {
                name,
                security,
                next: Mutex::new(first),
            })
        }

        /// Waits for a client on the waiting instance, makes the next one,
        /// and returns the connected one.
        pub fn accept(&self) -> io::Result<ServerStream> {
            let mut next = self.next.lock().unwrap();
            let handle = *next;
            let connected = unsafe { ConnectNamedPipe(handle, ptr::null_mut()) } != 0
                || unsafe { GetLastError() } == ERROR_PIPE_CONNECTED;
            let error = (!connected).then(io::Error::last_os_error);
            *next = create_instance(&self.name, &self.security, false)?;
            if let Some(error) = error {
                unsafe { CloseHandle(handle) };
                return Err(error);
            }
            Ok(unsafe { File::from_raw_handle(handle as _) })
        }
    }

    impl Drop for Listener {
        fn drop(&mut self) {
            unsafe { CloseHandle(*self.next.lock().unwrap()) };
        }
    }

    pub fn connect(address: &Path) -> io::Result<ClientStream> {
        let name = wide(address.as_os_str());
        // Every instance can be busy for a moment, while the agent makes the
        // next one; wait for one then. No pipe at all means no agent.
        for _ in 0..20 {
            let handle = unsafe {
                CreateFileW(
                    name.as_ptr(),
                    GENERIC_READ | GENERIC_WRITE,
                    0,
                    ptr::null(),
                    OPEN_EXISTING,
                    FILE_FLAG_OVERLAPPED,
                    ptr::null_mut(),
                )
            };
            if handle != INVALID_HANDLE_VALUE {
                return Ok(ClientStream(handle));
            }
            let error = io::Error::last_os_error();
            if error.raw_os_error() != Some(ERROR_PIPE_BUSY as i32) {
                return Err(error);
            }
            unsafe { WaitNamedPipeW(name.as_ptr(), 250) };
        }
        Err(io::Error::new(io::ErrorKind::TimedOut, "the agent is busy"))
    }

    /// The program at the other end of the pipe: the process that made it,
    /// and that process's executable.
    pub fn peer_program(stream: &ClientStream) -> Option<std::path::PathBuf> {
        use std::os::windows::ffi::OsStringExt;
        use windows_sys::Win32::System::Threading::{
            OpenProcess, QueryFullProcessImageNameW, PROCESS_QUERY_LIMITED_INFORMATION,
        };
        let pid = stream.server_process_id()?;
        let process = unsafe { OpenProcess(PROCESS_QUERY_LIMITED_INFORMATION, 0, pid) };
        if process.is_null() {
            return None;
        }
        let mut path = vec![0u16; 32_768];
        let mut length = path.len() as u32;
        let ok = unsafe { QueryFullProcessImageNameW(process, 0, path.as_mut_ptr(), &mut length) };
        unsafe { CloseHandle(process) };
        if ok == 0 {
            return None;
        }
        path.truncate(length as usize);
        Some(std::ffi::OsString::from_wide(&path).into())
    }

    /// The host's end. The host waits on the agent and the browser at once,
    /// from two threads, and Windows runs blocking I/O on one pipe handle one
    /// operation at a time, so a pending read would hold up every write. With
    /// overlapped I/O each thread has its own operation.
    pub struct ClientStream(HANDLE);

    unsafe impl Send for ClientStream {}

    impl ClientStream {
        /// The agent's process, which the host checks and lets bring prompts
        /// to the front.
        pub fn server_process_id(&self) -> Option<u32> {
            let mut pid = 0u32;
            (unsafe { GetNamedPipeServerProcessId(self.0, &mut pid) } != 0).then_some(pid)
        }

        pub fn try_clone(&self) -> io::Result<Self> {
            let process = unsafe { GetCurrentProcess() };
            let mut copy: HANDLE = ptr::null_mut();
            let ok = unsafe {
                DuplicateHandle(
                    process,
                    self.0,
                    process,
                    &mut copy,
                    0,
                    0,
                    DUPLICATE_SAME_ACCESS,
                )
            };
            if ok == 0 {
                return Err(io::Error::last_os_error());
            }
            Ok(Self(copy))
        }

        /// Starts one overlapped operation and waits for it on its own event.
        fn wait(&self, start: impl FnOnce(*mut OVERLAPPED) -> i32) -> io::Result<usize> {
            let event = unsafe { CreateEventW(ptr::null(), 1, 0, ptr::null()) };
            if event.is_null() {
                return Err(io::Error::last_os_error());
            }
            let mut overlapped: OVERLAPPED = unsafe { std::mem::zeroed() };
            overlapped.hEvent = event;
            let started =
                start(&mut overlapped) != 0 || unsafe { GetLastError() } == ERROR_IO_PENDING;
            let result = if !started {
                Err(io::Error::last_os_error())
            } else {
                let mut done = 0u32;
                if unsafe { GetOverlappedResult(self.0, &overlapped, &mut done, 1) } == 0 {
                    Err(io::Error::last_os_error())
                } else {
                    Ok(done as usize)
                }
            };
            unsafe { CloseHandle(event) };
            result
        }
    }

    impl io::Read for ClientStream {
        fn read(&mut self, buffer: &mut [u8]) -> io::Result<usize> {
            let length = buffer.len().min(u32::MAX as usize) as u32;
            let result = self.wait(|overlapped| unsafe {
                ReadFile(
                    self.0,
                    buffer.as_mut_ptr(),
                    length,
                    ptr::null_mut(),
                    overlapped,
                )
            });
            match result {
                // The agent closed its end: the end of the stream.
                Err(e) if e.raw_os_error() == Some(ERROR_BROKEN_PIPE as i32) => Ok(0),
                other => other,
            }
        }
    }

    impl io::Write for ClientStream {
        fn write(&mut self, buffer: &[u8]) -> io::Result<usize> {
            let length = buffer.len().min(u32::MAX as usize) as u32;
            self.wait(|overlapped| unsafe {
                WriteFile(self.0, buffer.as_ptr(), length, ptr::null_mut(), overlapped)
            })
        }

        fn flush(&mut self) -> io::Result<()> {
            Ok(())
        }
    }

    impl Drop for ClientStream {
        fn drop(&mut self) {
            unsafe { CloseHandle(self.0) };
        }
    }

    /// A security descriptor that lets only the current user and SYSTEM in.
    struct SecurityDescriptor(PSECURITY_DESCRIPTOR);

    impl SecurityDescriptor {
        fn current_user_only() -> io::Result<Self> {
            let sid = current_user_sid()?;
            // Protected DACL: full access for the user and SYSTEM, nobody else.
            let sddl = wide(OsStr::new(&format!("D:P(A;;GA;;;{sid})(A;;GA;;;SY)")));
            let mut descriptor: PSECURITY_DESCRIPTOR = ptr::null_mut();
            let ok = unsafe {
                ConvertStringSecurityDescriptorToSecurityDescriptorW(
                    sddl.as_ptr(),
                    SDDL_REVISION_1,
                    &mut descriptor,
                    ptr::null_mut(),
                )
            };
            if ok == 0 {
                return Err(io::Error::last_os_error());
            }
            Ok(Self(descriptor))
        }
    }

    impl Drop for SecurityDescriptor {
        fn drop(&mut self) {
            unsafe { LocalFree(self.0 as _) };
        }
    }

    fn current_user_sid() -> io::Result<String> {
        let mut token: HANDLE = ptr::null_mut();
        if unsafe { OpenProcessToken(GetCurrentProcess(), TOKEN_QUERY, &mut token) } == 0 {
            return Err(io::Error::last_os_error());
        }
        let mut buffer = vec![0u8; 256];
        let mut needed = 0u32;
        let ok = unsafe {
            GetTokenInformation(
                token,
                TokenUser,
                buffer.as_mut_ptr() as _,
                buffer.len() as u32,
                &mut needed,
            )
        };
        unsafe { CloseHandle(token) };
        if ok == 0 {
            return Err(io::Error::last_os_error());
        }
        let user = unsafe { &*(buffer.as_ptr() as *const TOKEN_USER) };
        let mut string: *mut u16 = ptr::null_mut();
        if unsafe { ConvertSidToStringSidW(user.User.Sid, &mut string) } == 0 {
            return Err(io::Error::last_os_error());
        }
        let length = (0..)
            .take_while(|&i| unsafe { *string.add(i) } != 0)
            .count();
        let sid = String::from_utf16_lossy(unsafe { std::slice::from_raw_parts(string, length) });
        unsafe { LocalFree(string as _) };
        Ok(sid)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[cfg(unix)]
    #[test]
    fn a_socket_another_program_made_isnt_the_agent() {
        let dir = std::env::temp_dir().join(format!("passbundle-ipc-{}", std::process::id()));
        let address = dir.join("agent.sock");
        let listener = Listener::bind(&address).unwrap();
        let client = connect(&address).unwrap();
        let _server = listener.accept().unwrap();
        // The other end is this test program, which the OS names.
        let peer = std::fs::canonicalize(unix::peer_program(&client).unwrap()).unwrap();
        assert_eq!(
            peer,
            std::fs::canonicalize(std::env::current_exe().unwrap()).unwrap()
        );
        assert!(!reached_installed_agent(&client));
        let _ = std::fs::remove_dir_all(dir);
    }

    #[test]
    fn frames_round_trip_and_oversized_ones_are_refused() {
        let mut buffer = Vec::new();
        write_frame(&mut buffer, b"{\"command\":\"connected\"}").unwrap();
        write_frame(&mut buffer, b"").unwrap();
        let mut reader = io::Cursor::new(buffer);
        assert_eq!(
            read_frame(&mut reader).unwrap().unwrap(),
            b"{\"command\":\"connected\"}"
        );
        assert_eq!(read_frame(&mut reader).unwrap().unwrap(), b"");
        assert!(read_frame(&mut reader).unwrap().is_none());

        let mut huge = ((MAX_FRAME + 1) as u32).to_ne_bytes().to_vec();
        huge.extend([0u8; 16]);
        assert!(read_frame(&mut io::Cursor::new(huge)).is_err());
        assert!(write_frame(&mut Vec::new(), &vec![0u8; MAX_FRAME + 1]).is_err());
    }

    #[test]
    fn each_user_gets_their_own_address() {
        let a = address_for(std::ffi::OsStr::new("/Users/ana"));
        let b = address_for(std::ffi::OsStr::new("/Users/ben"));
        assert_ne!(a, b);
    }

    #[test]
    fn the_host_and_the_agent_talk_over_the_socket() {
        // A short folder: Unix socket paths are limited to about 100 bytes, and
        // macOS's temporary folder is long.
        let base = if cfg!(unix) {
            PathBuf::from("/tmp")
        } else {
            std::env::temp_dir()
        };
        let home = base.join(format!("pb-ipc-{}", std::process::id()));
        let address = address_for(home.as_os_str());
        let listener = Listener::bind(&address).unwrap();
        let server = std::thread::spawn(move || {
            let mut stream = listener.accept().unwrap();
            let frame = read_frame(&mut stream).unwrap().unwrap();
            write_frame(&mut stream, &[b"echo: ".as_slice(), &frame].concat()).unwrap();
        });
        let mut client = connect(&address).unwrap();
        write_frame(&mut client, b"hello").unwrap();
        assert_eq!(read_frame(&mut client).unwrap().unwrap(), b"echo: hello");
        server.join().unwrap();
        let _ = std::fs::remove_dir_all(home);
    }
}
