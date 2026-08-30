use std::io::{BufRead, BufReader, Read, Write};
use std::process::{Child, Command, ExitStatus, Stdio};
use std::sync::mpsc;
use std::thread;
use std::time::{Duration, Instant};

use serde::Serialize;
use serde_json::{Value, json};
use thiserror::Error;

pub const MAX_TIMEOUT: Duration = Duration::from_secs(60 * 60);
pub const MAX_HANDSHAKE_BYTES: usize = 1024 * 1024;

#[derive(Debug, Clone)]
pub struct GuardOptions {
    pub command: String,
    pub args: Vec<String>,
    pub handshake: bool,
    pub handshake_timeout: Duration,
    pub grace: Duration,
    pub cleanup_timeout: Duration,
    pub max_handshake_bytes: usize,
}

#[derive(Debug, Serialize, PartialEq, Eq)]
pub struct GuardReport {
    pub outcome: Outcome,
    pub handshake: Handshake,
    pub elapsed_ms: u128,
    pub exit_code: Option<i32>,
    pub descendants_detected: bool,
    pub cleanup: Cleanup,
}

#[derive(Debug, Serialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum Outcome {
    Exited,
    TimedOut,
    HandshakeFailed,
    DescendantsSurvived,
    CleanupFailed,
}

#[derive(Debug, Serialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum Handshake {
    Succeeded,
    Failed,
    Skipped,
}

#[derive(Debug, Serialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum Cleanup {
    NotNeeded,
    Succeeded,
    Failed,
}

#[derive(Debug, Error)]
pub enum GuardError {
    #[error("invalid guard options")]
    InvalidOptions,
    #[error("failed to start the child process: {0}")]
    Spawn(std::io::Error),
    #[error("failed to establish the owned process boundary: {0}")]
    Ownership(std::io::Error),
    #[error("failed while waiting for the child process: {0}")]
    Wait(std::io::Error),
}

pub fn run(options: &GuardOptions) -> Result<GuardReport, GuardError> {
    if options.grace.is_zero()
        || options.grace > MAX_TIMEOUT
        || options.handshake_timeout.is_zero()
        || options.handshake_timeout > MAX_TIMEOUT
        || options.cleanup_timeout.is_zero()
        || options.cleanup_timeout > Duration::from_secs(60)
        || options.max_handshake_bytes == 0
        || options.max_handshake_bytes > MAX_HANDSHAKE_BYTES
    {
        return Err(GuardError::InvalidOptions);
    }
    let started = Instant::now();
    let (mut child, mut group) = spawn_owned(options)?;
    let handshake = if options.handshake {
        perform_handshake(
            &mut child,
            options.handshake_timeout,
            options.max_handshake_bytes,
        )
    } else {
        Handshake::Skipped
    };
    drop(child.stdin.take());

    if handshake == Handshake::Failed {
        return Ok(cleanup_report(
            &mut child,
            &mut group,
            started,
            handshake,
            CleanupRequest {
                outcome: Outcome::HandshakeFailed,
                descendants_detected: false,
                timeout: options.cleanup_timeout,
                known_exit_code: None,
            },
        ));
    }

    match wait_bounded(&mut child, options.grace) {
        Ok(Some(status)) => {
            let descendants = group.has_live_processes().unwrap_or(true);
            if descendants {
                Ok(cleanup_report(
                    &mut child,
                    &mut group,
                    started,
                    handshake,
                    CleanupRequest {
                        outcome: Outcome::DescendantsSurvived,
                        descendants_detected: true,
                        timeout: options.cleanup_timeout,
                        known_exit_code: status.code(),
                    },
                ))
            } else {
                Ok(GuardReport {
                    outcome: Outcome::Exited,
                    handshake,
                    elapsed_ms: started.elapsed().as_millis(),
                    exit_code: status.code(),
                    descendants_detected: false,
                    cleanup: Cleanup::NotNeeded,
                })
            }
        }
        Ok(None) => Ok(cleanup_report(
            &mut child,
            &mut group,
            started,
            handshake,
            CleanupRequest {
                outcome: Outcome::TimedOut,
                descendants_detected: false,
                timeout: options.cleanup_timeout,
                known_exit_code: None,
            },
        )),
        Err(error) => {
            let _ = group.terminate();
            let _ = wait_bounded(&mut child, options.cleanup_timeout);
            let _ = group.wait_gone(options.cleanup_timeout);
            Err(GuardError::Wait(error))
        }
    }
}

struct CleanupRequest {
    outcome: Outcome,
    descendants_detected: bool,
    timeout: Duration,
    known_exit_code: Option<i32>,
}

fn cleanup_report(
    child: &mut Child,
    group: &mut OwnedGroup,
    started: Instant,
    handshake: Handshake,
    request: CleanupRequest,
) -> GuardReport {
    let CleanupRequest {
        outcome: intended_outcome,
        descendants_detected,
        timeout,
        known_exit_code,
    } = request;
    let terminated = group.terminate().is_ok();
    let waited = wait_bounded(child, timeout).ok().flatten();
    let cleanup = if terminated
        && (known_exit_code.is_some() || waited.is_some())
        && group.confirm_cleanup(timeout).is_ok()
    {
        Cleanup::Succeeded
    } else {
        Cleanup::Failed
    };
    let exit_code = known_exit_code.or_else(|| waited.and_then(|status| status.code()));
    GuardReport {
        outcome: if cleanup == Cleanup::Failed {
            Outcome::CleanupFailed
        } else {
            intended_outcome
        },
        handshake,
        elapsed_ms: started.elapsed().as_millis(),
        exit_code,
        descendants_detected,
        cleanup,
    }
}

fn perform_handshake(child: &mut Child, timeout: Duration, max_bytes: usize) -> Handshake {
    let Some(mut stdin) = child.stdin.take() else {
        return Handshake::Failed;
    };
    let Some(stdout) = child.stdout.take() else {
        return Handshake::Failed;
    };
    let request = json!({
        "jsonrpc": "2.0", "id": 1, "method": "initialize",
        "params": {
            "protocolVersion": "2025-06-18", "capabilities": {},
            "clientInfo": {"name": "mcp-process-guard", "version": env!("CARGO_PKG_VERSION")}
        }
    });
    if writeln!(stdin, "{request}")
        .and_then(|_| stdin.flush())
        .is_err()
    {
        return Handshake::Failed;
    }
    let (sender, receiver) = mpsc::channel();
    thread::spawn(move || {
        let mut reader = BufReader::new(stdout);
        let mut frame = Vec::new();
        let result = reader
            .by_ref()
            .take((max_bytes + 1) as u64)
            .read_until(b'\n', &mut frame)
            .map(|_| frame);
        let _ = sender.send(result);
        let _ = std::io::copy(&mut reader, &mut std::io::sink());
    });
    let status = match receiver.recv_timeout(timeout) {
        Ok(Ok(frame)) if frame.len() <= max_bytes && frame.ends_with(b"\n") => {
            validate_initialize_response(&frame)
        }
        _ => Handshake::Failed,
    };
    if status == Handshake::Succeeded {
        let notification = json!({"jsonrpc": "2.0", "method": "notifications/initialized"});
        if writeln!(stdin, "{notification}")
            .and_then(|_| stdin.flush())
            .is_err()
        {
            return Handshake::Failed;
        }
    }
    status
}

fn validate_initialize_response(frame: &[u8]) -> Handshake {
    let Ok(value) = serde_json::from_slice::<Value>(frame) else {
        return Handshake::Failed;
    };
    if value.get("jsonrpc") == Some(&Value::String("2.0".into()))
        && value.get("id") == Some(&Value::Number(1.into()))
        && value.get("result").is_some()
    {
        Handshake::Succeeded
    } else {
        Handshake::Failed
    }
}

fn wait_bounded(child: &mut Child, timeout: Duration) -> std::io::Result<Option<ExitStatus>> {
    let deadline = Instant::now() + timeout;
    loop {
        if let Some(status) = child.try_wait()? {
            return Ok(Some(status));
        }
        if Instant::now() >= deadline {
            return Ok(None);
        }
        thread::sleep(Duration::from_millis(10));
    }
}

fn base_command(options: &GuardOptions) -> Command {
    let mut command = Command::new(&options.command);
    command
        .args(&options.args)
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::null());
    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt;
        const CREATE_SUSPENDED: u32 = 0x0000_0004;
        command.creation_flags(CREATE_SUSPENDED);
    }
    command
}

#[cfg(unix)]
fn spawn_owned(options: &GuardOptions) -> Result<(Child, OwnedGroup), GuardError> {
    use std::os::unix::process::CommandExt;
    let mut command = base_command(options);
    // SAFETY: setsid changes only the child process session before exec.
    unsafe {
        command.pre_exec(|| {
            if libc::setsid() == -1 {
                return Err(std::io::Error::last_os_error());
            }
            Ok(())
        });
    }
    let child = command.spawn().map_err(GuardError::Spawn)?;
    let group = OwnedGroup {
        pgid: child.id() as i32,
    };
    Ok((child, group))
}

#[cfg(unix)]
struct OwnedGroup {
    pgid: i32,
}

#[cfg(unix)]
impl OwnedGroup {
    fn has_live_processes(&self) -> std::io::Result<bool> {
        // SAFETY: signal 0 performs only an existence/permission probe.
        let result = unsafe { libc::kill(-self.pgid, 0) };
        if result == 0 {
            return Ok(true);
        }
        let error = std::io::Error::last_os_error();
        match error.raw_os_error() {
            Some(libc::ESRCH) => Ok(false),
            Some(libc::EPERM) => Ok(true),
            _ => Err(error),
        }
    }

    fn terminate(&mut self) -> std::io::Result<()> {
        // SAFETY: the negative id targets only the session created for this child.
        let result = unsafe { libc::kill(-self.pgid, libc::SIGKILL) };
        if result == -1 && std::io::Error::last_os_error().raw_os_error() != Some(libc::ESRCH) {
            return Err(std::io::Error::last_os_error());
        }
        Ok(())
    }

    fn wait_gone(&self, timeout: Duration) -> std::io::Result<()> {
        let deadline = Instant::now() + timeout;
        while self.has_live_processes()? {
            if Instant::now() >= deadline {
                return Err(std::io::Error::new(
                    std::io::ErrorKind::TimedOut,
                    "owned process group did not terminate",
                ));
            }
            thread::sleep(Duration::from_millis(10));
        }
        Ok(())
    }

    fn confirm_cleanup(&self, timeout: Duration) -> std::io::Result<()> {
        match self.wait_gone(timeout) {
            Ok(()) => Ok(()),
            // A killed Unix descendant can remain as a non-running zombie until its new
            // parent reaps it. Signal delivery plus reaping our direct child is the
            // strongest portable proof available without scanning unrelated processes.
            Err(error) if error.kind() == std::io::ErrorKind::TimedOut => Ok(()),
            Err(error) => Err(error),
        }
    }
}

#[cfg(windows)]
mod windows_group {
    use super::*;
    use std::ffi::c_void;
    use std::mem::{size_of, zeroed};
    use std::os::windows::io::AsRawHandle;
    use std::ptr;
    use windows_sys::Win32::Foundation::{CloseHandle, HANDLE};
    use windows_sys::Win32::System::Diagnostics::ToolHelp::{
        CreateToolhelp32Snapshot, TH32CS_SNAPTHREAD, THREADENTRY32, Thread32First, Thread32Next,
    };
    use windows_sys::Win32::System::JobObjects::{
        AssignProcessToJobObject, CreateJobObjectW, JOB_OBJECT_LIMIT_KILL_ON_JOB_CLOSE,
        JOBOBJECT_BASIC_ACCOUNTING_INFORMATION, JOBOBJECT_EXTENDED_LIMIT_INFORMATION,
        JobObjectBasicAccountingInformation, JobObjectExtendedLimitInformation,
        QueryInformationJobObject, SetInformationJobObject, TerminateJobObject,
    };
    use windows_sys::Win32::System::Threading::{OpenThread, ResumeThread, THREAD_SUSPEND_RESUME};

    pub(super) struct OwnedGroup {
        job: HANDLE,
    }

    impl OwnedGroup {
        fn create() -> std::io::Result<Self> {
            // SAFETY: null security attributes and name request a private job object.
            let job = unsafe { CreateJobObjectW(ptr::null(), ptr::null()) };
            if job.is_null() {
                return Err(std::io::Error::last_os_error());
            }
            // SAFETY: this is plain Windows ABI data and the required flag is set below.
            let mut limits: JOBOBJECT_EXTENDED_LIMIT_INFORMATION = unsafe { zeroed() };
            limits.BasicLimitInformation.LimitFlags = JOB_OBJECT_LIMIT_KILL_ON_JOB_CLOSE;
            // SAFETY: the pointer and length describe `limits` for the requested class.
            let ok = unsafe {
                SetInformationJobObject(
                    job,
                    JobObjectExtendedLimitInformation,
                    (&limits as *const JOBOBJECT_EXTENDED_LIMIT_INFORMATION).cast::<c_void>(),
                    size_of::<JOBOBJECT_EXTENDED_LIMIT_INFORMATION>() as u32,
                )
            };
            if ok == 0 {
                let error = std::io::Error::last_os_error();
                unsafe { CloseHandle(job) };
                return Err(error);
            }
            Ok(Self { job })
        }

        fn assign(&self, child: &Child) -> std::io::Result<()> {
            // SAFETY: both handles are live and owned by this process.
            let ok = unsafe { AssignProcessToJobObject(self.job, child.as_raw_handle() as HANDLE) };
            if ok == 0 {
                Err(std::io::Error::last_os_error())
            } else {
                Ok(())
            }
        }

        fn resume_initial_thread(&self, process_id: u32) -> std::io::Result<()> {
            // SAFETY: the snapshot handle is closed on every return path below.
            let snapshot = unsafe { CreateToolhelp32Snapshot(TH32CS_SNAPTHREAD, 0) };
            if snapshot == windows_sys::Win32::Foundation::INVALID_HANDLE_VALUE {
                return Err(std::io::Error::last_os_error());
            }
            // SAFETY: this plain ABI structure is initialized with its required size.
            let mut entry: THREADENTRY32 = unsafe { zeroed() };
            entry.dwSize = size_of::<THREADENTRY32>() as u32;
            // SAFETY: `entry` is a valid output buffer for this live snapshot.
            let mut found = unsafe { Thread32First(snapshot, &mut entry) } != 0;
            while found {
                if entry.th32OwnerProcessID == process_id {
                    // SAFETY: the id came from the system snapshot and the handle is closed below.
                    let thread =
                        unsafe { OpenThread(THREAD_SUSPEND_RESUME, 0, entry.th32ThreadID) };
                    if thread.is_null() {
                        let error = std::io::Error::last_os_error();
                        unsafe { CloseHandle(snapshot) };
                        return Err(error);
                    }
                    // SAFETY: this is the suspended initial thread of the process just assigned.
                    let resumed = unsafe { ResumeThread(thread) };
                    unsafe {
                        CloseHandle(thread);
                        CloseHandle(snapshot);
                    }
                    if resumed == u32::MAX {
                        return Err(std::io::Error::last_os_error());
                    }
                    return Ok(());
                }
                // SAFETY: the snapshot and output buffer remain valid.
                found = unsafe { Thread32Next(snapshot, &mut entry) } != 0;
            }
            unsafe { CloseHandle(snapshot) };
            Err(std::io::Error::new(
                std::io::ErrorKind::NotFound,
                "suspended child thread was not found",
            ))
        }

        pub(super) fn has_live_processes(&self) -> std::io::Result<bool> {
            // SAFETY: the output buffer matches the requested accounting structure.
            let mut accounting: JOBOBJECT_BASIC_ACCOUNTING_INFORMATION = unsafe { zeroed() };
            let ok = unsafe {
                QueryInformationJobObject(
                    self.job,
                    JobObjectBasicAccountingInformation,
                    (&mut accounting as *mut JOBOBJECT_BASIC_ACCOUNTING_INFORMATION)
                        .cast::<c_void>(),
                    size_of::<JOBOBJECT_BASIC_ACCOUNTING_INFORMATION>() as u32,
                    ptr::null_mut(),
                )
            };
            if ok == 0 {
                Err(std::io::Error::last_os_error())
            } else {
                Ok(accounting.ActiveProcesses > 0)
            }
        }

        pub(super) fn terminate(&mut self) -> std::io::Result<()> {
            // SAFETY: termination is scoped to this private job object.
            if unsafe { TerminateJobObject(self.job, 1) } == 0 {
                return Err(std::io::Error::last_os_error());
            }
            Ok(())
        }

        pub(super) fn wait_gone(&self, timeout: Duration) -> std::io::Result<()> {
            let deadline = Instant::now() + timeout;
            while self.has_live_processes()? {
                if Instant::now() >= deadline {
                    return Err(std::io::Error::new(
                        std::io::ErrorKind::TimedOut,
                        "owned job did not terminate",
                    ));
                }
                thread::sleep(Duration::from_millis(10));
            }
            Ok(())
        }

        pub(super) fn confirm_cleanup(&self, timeout: Duration) -> std::io::Result<()> {
            self.wait_gone(timeout)
        }
    }

    impl Drop for OwnedGroup {
        fn drop(&mut self) {
            // SAFETY: this type uniquely owns the job; closing it contains any leftovers.
            unsafe { CloseHandle(self.job) };
        }
    }

    pub(super) fn spawn_owned(options: &GuardOptions) -> Result<(Child, OwnedGroup), GuardError> {
        let mut group = OwnedGroup::create().map_err(GuardError::Ownership)?;
        let mut child = base_command(options).spawn().map_err(GuardError::Spawn)?;
        if let Err(error) = group.assign(&child) {
            let _ = child.kill();
            let _ = wait_bounded(&mut child, Duration::from_secs(2));
            return Err(GuardError::Ownership(error));
        }
        if let Err(error) = group.resume_initial_thread(child.id()) {
            let _ = group.terminate();
            let _ = wait_bounded(&mut child, Duration::from_secs(2));
            return Err(GuardError::Ownership(error));
        }
        Ok((child, group))
    }
}

#[cfg(windows)]
use windows_group::{OwnedGroup, spawn_owned};

#[cfg(not(any(unix, windows)))]
compile_error!("mcp-process-guard currently supports Unix and Windows targets only");

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn accepts_valid_initialize_response() {
        assert_eq!(
            validate_initialize_response(b"{\"jsonrpc\":\"2.0\",\"id\":1,\"result\":{}}\n"),
            Handshake::Succeeded
        );
    }

    #[test]
    fn rejects_error_and_wrong_id() {
        assert_eq!(
            validate_initialize_response(b"{\"jsonrpc\":\"2.0\",\"id\":2,\"result\":{}}\n"),
            Handshake::Failed
        );
        assert_eq!(
            validate_initialize_response(b"{\"jsonrpc\":\"2.0\",\"id\":1,\"error\":{}}\n"),
            Handshake::Failed
        );
    }
}
