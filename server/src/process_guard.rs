//! Makes child processes (llama-server, agent commands) end when Relay ends,
//! even if Relay is killed without a clean shutdown (Task Manager, a crash).
//!
//! On Windows every child is put in a job object that kills its processes
//! when the last handle to it closes, which happens when Relay's process
//! exits for any reason. Elsewhere this is a no-op; Relay's normal shutdown
//! still stops its children.

#[cfg(windows)]
mod imp {
    use std::ffi::c_void;
    use std::sync::OnceLock;

    use windows_sys::Win32::Foundation::HANDLE;
    use windows_sys::Win32::System::JobObjects::{
        AssignProcessToJobObject, CreateJobObjectW, JOB_OBJECT_LIMIT_KILL_ON_JOB_CLOSE,
        JOBOBJECT_EXTENDED_LIMIT_INFORMATION, JobObjectExtendedLimitInformation,
        SetInformationJobObject,
    };

    struct Job(HANDLE);
    // SAFETY: a job handle may be used from any thread; it is never closed
    // (it must live as long as the process).
    unsafe impl Send for Job {}
    unsafe impl Sync for Job {}

    fn job() -> Option<HANDLE> {
        static JOB: OnceLock<Option<Job>> = OnceLock::new();
        JOB.get_or_init(|| {
            // SAFETY: plain Win32 calls with valid arguments; the info struct
            // is zero-initialised and lives for the duration of the call.
            unsafe {
                let job = CreateJobObjectW(std::ptr::null(), std::ptr::null());
                if job.is_null() {
                    return None;
                }
                let mut info: JOBOBJECT_EXTENDED_LIMIT_INFORMATION = std::mem::zeroed();
                info.BasicLimitInformation.LimitFlags = JOB_OBJECT_LIMIT_KILL_ON_JOB_CLOSE;
                let ok = SetInformationJobObject(
                    job,
                    JobObjectExtendedLimitInformation,
                    &info as *const _ as *const c_void,
                    std::mem::size_of::<JOBOBJECT_EXTENDED_LIMIT_INFORMATION>() as u32,
                );
                if ok == 0 {
                    return None;
                }
                Some(Job(job))
            }
        })
        .as_ref()
        .map(|j| j.0)
    }

    pub fn tie_to_relay(child: &tokio::process::Child) {
        let (Some(job), Some(process)) = (job(), child.raw_handle()) else {
            tracing::debug!("could not tie child process to Relay's lifetime");
            return;
        };
        // SAFETY: both handles are valid for the duration of the call.
        let ok = unsafe { AssignProcessToJobObject(job, process as HANDLE) };
        if ok == 0 {
            tracing::debug!("AssignProcessToJobObject failed");
        }
    }
}

#[cfg(not(windows))]
mod imp {
    pub fn tie_to_relay(_child: &tokio::process::Child) {}
}

pub use imp::tie_to_relay;
