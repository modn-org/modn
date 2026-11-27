//! Windows job-object tree confinement.
//!
//! The child starts with `CREATE_SUSPENDED`. Modn creates a job object with
//! `JOB_OBJECT_LIMIT_KILL_ON_JOB_CLOSE`, assigns the suspended child to it and
//! only then resumes the child's threads. A process that is suspended cannot
//! start a grandchild before the assignment, so every descendant is inside the
//! job and `TerminateJobObject` reaches all of them.
//!
//! `std::process::Command` exposes no thread handle, so the resume uses a
//! Toolhelp thread snapshot of the child's process ID. `ResumeThread` on each
//! listed thread is the documented Microsoft mechanism; `NtResumeProcess` is
//! an undocumented native call and is not used.
#![expect(
    unsafe_code,
    reason = "the Windows job-object, Toolhelp and process bindings are FFI and have no safe wrapper"
)]

use super::{wait_until_empty, TreeError};
use std::io;
use std::mem::size_of;
use std::os::windows::io::AsRawHandle;
use std::os::windows::process::CommandExt;
use std::process::{Child, Command};
use std::ptr;
use windows_sys::Win32::Foundation::{CloseHandle, HANDLE, INVALID_HANDLE_VALUE};
use windows_sys::Win32::System::Diagnostics::ToolHelp::{
    CreateToolhelp32Snapshot, Thread32First, Thread32Next, TH32CS_SNAPTHREAD, THREADENTRY32,
};
use windows_sys::Win32::System::JobObjects::{
    AssignProcessToJobObject, CreateJobObjectW, JobObjectBasicAccountingInformation,
    JobObjectExtendedLimitInformation, QueryInformationJobObject, SetInformationJobObject,
    TerminateJobObject, JOBOBJECT_BASIC_ACCOUNTING_INFORMATION,
    JOBOBJECT_EXTENDED_LIMIT_INFORMATION, JOB_OBJECT_LIMIT_KILL_ON_JOB_CLOSE,
};
use windows_sys::Win32::System::Threading::{
    OpenThread, ResumeThread, CREATE_SUSPENDED, THREAD_SUSPEND_RESUME,
};

/// One job object that holds every process of the Git tree.
pub(crate) struct Tree {
    job: HANDLE,
}

impl Tree {
    /// Start `command` suspended so the job assignment happens first.
    pub(crate) fn confine(command: &mut Command) -> io::Result<()> {
        command.creation_flags(CREATE_SUSPENDED);
        Ok(())
    }

    /// Create the job, assign the suspended child and resume its threads.
    pub(crate) fn attach(child: &Child) -> Result<Self, TreeError> {
        // SAFETY: The null security attributes and name ask for an unnamed job
        // with default security. The returned handle is owned by `Tree`.
        let job = unsafe { CreateJobObjectW(ptr::null(), ptr::null()) };
        if job.is_null() {
            return Err(TreeError::Io(io::Error::last_os_error()));
        }
        let tree = Self { job };
        // SAFETY: `job` is a live job handle. The information struct is fully
        // initialized and its size is passed with the pointer.
        // SAFETY: A zeroed extended-limit struct is a valid value; its fields
        // are integers and the limit flags are set before it is passed on.
        let mut limits: JOBOBJECT_EXTENDED_LIMIT_INFORMATION = unsafe { std::mem::zeroed() };
        limits.BasicLimitInformation.LimitFlags = JOB_OBJECT_LIMIT_KILL_ON_JOB_CLOSE;
        // SAFETY: `job` is a live job handle. The information struct is fully
        // initialized and its size is passed with the pointer.
        let configured = unsafe {
            SetInformationJobObject(
                tree.job,
                JobObjectExtendedLimitInformation,
                &limits as *const _ as *const core::ffi::c_void,
                size_of::<JOBOBJECT_EXTENDED_LIMIT_INFORMATION>() as u32,
            )
        };
        if configured == 0 {
            return Err(TreeError::Io(io::Error::last_os_error()));
        }
        // SAFETY: The child handle from `CreateProcessW` has full process
        // access, which includes the set-quota and terminate rights that
        // `AssignProcessToJobObject` requires.
        if unsafe { AssignProcessToJobObject(tree.job, child.as_raw_handle()) } == 0 {
            return Err(TreeError::Io(io::Error::last_os_error()));
        }
        resume_suspended(child.id())?;
        Ok(tree)
    }

    /// Terminate the job, reap the direct child and confirm it is empty.
    pub(crate) fn stop(&self, child: &mut Child) -> Result<(), TreeError> {
        self.terminate()?;
        child.wait().map_err(TreeError::Io)?;
        self.confirm_empty()
    }

    /// Terminate what is left after the direct child exited on its own.
    pub(crate) fn finish(&self) -> Result<(), TreeError> {
        self.terminate()?;
        self.confirm_empty()
    }

    /// Terminate without waiting. Only a dropping guard uses this.
    pub(crate) fn abandon(&self) {
        // SAFETY: `self.job` is a live job handle until `Tree` drops.
        unsafe {
            TerminateJobObject(self.job, 1);
        }
    }

    fn terminate(&self) -> Result<(), TreeError> {
        // SAFETY: `self.job` is a live job handle until `Tree` drops.
        if unsafe { TerminateJobObject(self.job, 1) } == 0 {
            return Err(TreeError::Io(io::Error::last_os_error()));
        }
        Ok(())
    }

    fn confirm_empty(&self) -> Result<(), TreeError> {
        wait_until_empty(|| Ok(self.active_processes()? == 0))
    }

    fn active_processes(&self) -> io::Result<u32> {
        // SAFETY: `self.job` is live, and the accounting struct is fully
        // initialized before the call fills it.
        // SAFETY: A zeroed accounting struct is a valid value; the call fills
        // it before any field is read.
        let mut accounting: JOBOBJECT_BASIC_ACCOUNTING_INFORMATION = unsafe { std::mem::zeroed() };
        // SAFETY: `self.job` is live, the pointer is writable, and the size
        // matches the accounting struct.
        let queried = unsafe {
            QueryInformationJobObject(
                self.job,
                JobObjectBasicAccountingInformation,
                &mut accounting as *mut _ as *mut core::ffi::c_void,
                size_of::<JOBOBJECT_BASIC_ACCOUNTING_INFORMATION>() as u32,
                ptr::null_mut(),
            )
        };
        if queried == 0 {
            return Err(io::Error::last_os_error());
        }
        Ok(accounting.ActiveProcesses)
    }
}

impl Drop for Tree {
    fn drop(&mut self) {
        // SAFETY: The handle is owned by this value and is closed once.
        // `JOB_OBJECT_LIMIT_KILL_ON_JOB_CLOSE` stops any process still inside.
        unsafe {
            CloseHandle(self.job);
        }
    }
}

/// Resume every thread of the suspended process `pid`.
fn resume_suspended(pid: u32) -> Result<(), TreeError> {
    // SAFETY: A snapshot handle is returned for the system thread list.
    let snapshot = unsafe { CreateToolhelp32Snapshot(TH32CS_SNAPTHREAD, 0) };
    if snapshot == INVALID_HANDLE_VALUE {
        return Err(TreeError::Io(io::Error::last_os_error()));
    }
    let result = resume_threads(snapshot, pid);
    // SAFETY: The snapshot handle is live and is closed once here.
    unsafe {
        CloseHandle(snapshot);
    }
    result
}

fn resume_threads(snapshot: HANDLE, pid: u32) -> Result<(), TreeError> {
    // SAFETY: The entry is a plain data struct; its size field must be set
    // before `Thread32First`.
    let mut entry: THREADENTRY32 = unsafe { std::mem::zeroed() };
    entry.dwSize = size_of::<THREADENTRY32>() as u32;
    // SAFETY: `snapshot` is a live snapshot handle and `entry` is writable.
    if unsafe { Thread32First(snapshot, &mut entry) } == 0 {
        return Err(TreeError::Io(io::Error::last_os_error()));
    }
    let mut resumed = 0_u32;
    loop {
        if entry.th32OwnerProcessID == pid {
            // SAFETY: The thread ID comes from the snapshot above.
            let thread = unsafe { OpenThread(THREAD_SUSPEND_RESUME, 0, entry.th32ThreadID) };
            if thread.is_null() {
                return Err(TreeError::Io(io::Error::last_os_error()));
            }
            // SAFETY: `thread` is a live thread handle with resume access.
            let previous = unsafe { ResumeThread(thread) };
            // SAFETY: The thread handle is live and is closed once here.
            unsafe {
                CloseHandle(thread);
            }
            if previous == u32::MAX {
                return Err(TreeError::Io(io::Error::last_os_error()));
            }
            resumed += 1;
        }
        // SAFETY: `snapshot` and `entry` are valid for the next iteration.
        if unsafe { Thread32Next(snapshot, &mut entry) } == 0 {
            break;
        }
    }
    if resumed == 0 {
        return Err(TreeError::Io(io::Error::new(
            io::ErrorKind::NotFound,
            format!("the suspended Git process {pid} exposed no thread to resume"),
        )));
    }
    Ok(())
}
