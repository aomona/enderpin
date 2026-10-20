//! Scoped interactive window-station access for GLFW cursor confinement.
use super::*;
use windows_sys::Win32::{
    Foundation::WAIT_ABANDONED,
    Security::{
        Authorization::{
            EXPLICIT_ACCESS_W, GRANT_ACCESS, GetSecurityInfo, REVOKE_ACCESS, SE_WINDOW_OBJECT,
            SetEntriesInAclW, SetSecurityInfo, TRUSTEE_IS_SID, TRUSTEE_W,
        },
        DACL_SECURITY_INFORMATION,
    },
    System::{
        StationsAndDesktops::GetProcessWindowStation,
        Threading::{CreateMutexW, ReleaseMutex},
    },
    UI::WindowsAndMessaging::WINSTA_WRITEATTRIBUTES,
};

// Serialize read/modify/write updates across Enderpin processes in this session.
// Never restore a whole saved DACL: another live client's grant must survive.
struct AclLock(OwnedHandle);
impl AclLock {
    fn acquire() -> Result<Self> {
        let name = wide(OsStr::new("Local\\Enderpin.CursorAcl.v1"))?;
        // SAFETY: the name is terminated and no security descriptor is supplied.
        let handle = owned(unsafe { CreateMutexW(null(), 0, name.as_ptr()) })?;
        // SAFETY: handle is a live mutex. An abandoned owner made no partial ACL
        // writes: SetSecurityInfo replaces the DACL in one system call.
        let result = unsafe { WaitForSingleObject(handle.as_raw_handle(), 5000) };
        ensure!(
            result == WAIT_OBJECT_0 || result == WAIT_ABANDONED,
            "cannot acquire cursor permission lock: {result}"
        );
        Ok(Self(handle))
    }
}
impl Drop for AclLock {
    fn drop(&mut self) {
        // SAFETY: this thread acquired the mutex and has not released it yet.
        unsafe {
            ReleaseMutex(self.0.as_raw_handle());
        }
    }
}

pub(super) struct Grant {
    sid: Local,
    station: HANDLE,
    registered: bool,
}
// SAFETY: SID memory is immutable and owned, window-station handles are usable
// across threads, and all ACL updates are serialized by AclLock.
unsafe impl Send for Grant {}
unsafe impl Sync for Grant {}

impl Grant {
    pub fn new() -> Result<Self> {
        // SAFETY: borrows this process's station; no ownership is transferred.
        Self::for_station(unsafe { GetProcessWindowStation() })
    }

    fn for_station(station: HANDLE) -> Result<Self> {
        ensure!(!station.is_null(), "interactive window station is missing");
        // Use an unguessable, per-launch capability, not ALL APPLICATION PACKAGES
        // or a persistent profile SID. A leftover ACE after a launcher crash
        // cannot grant cursor authority to a subsequent launch.
        let mut random = [0u8; 32];
        getrandom::fill(&mut random)
            .map_err(|_| anyhow::anyhow!("cursor capability randomness unavailable"))?;
        let suffix: String = random
            .chunks_exact(4)
            .map(|v| format!("-{}", u32::from_le_bytes(v.try_into().expect("four bytes"))))
            .collect();
        let text = wide(OsStr::new(&format!("S-1-15-3-1024{suffix}")))?;
        let mut sid = null_mut();
        // SAFETY: terminated SID text and valid output pointer; SDK allocates SID.
        check(unsafe { ConvertStringSidToSidW(text.as_ptr(), &mut sid) })?;
        let mut grant = Self {
            sid: Local(sid),
            station,
            registered: false,
        };
        grant.update(GRANT_ACCESS)?;
        grant.registered = true;
        Ok(grant)
    }

    pub fn sid(&self) -> *mut c_void {
        self.sid.0
    }

    fn update(&self, mode: i32) -> Result<()> {
        let _lock = AclLock::acquire()?;
        let mut acl = null_mut();
        let mut descriptor = null_mut();
        // SAFETY: station and output pointers are live; only the DACL is read.
        let result = unsafe {
            GetSecurityInfo(
                self.station,
                SE_WINDOW_OBJECT,
                DACL_SECURITY_INFORMATION,
                null_mut(),
                null_mut(),
                &mut acl,
                null_mut(),
                &mut descriptor,
            )
        };
        ensure!(
            result == 0,
            "cannot read cursor permissions: Windows error {result}"
        );
        let _descriptor = Local(descriptor);
        ensure!(!acl.is_null(), "window-station DACL is missing");
        let entry = EXPLICIT_ACCESS_W {
            grfAccessPermissions: WINSTA_WRITEATTRIBUTES as u32,
            grfAccessMode: mode,
            grfInheritance: 0,
            Trustee: TRUSTEE_W {
                TrusteeForm: TRUSTEE_IS_SID,
                ptstrName: self.sid.0.cast(),
                ..Default::default()
            },
        };
        let mut modified = null_mut();
        // SAFETY: entry contains our live SID and acl is owned by _descriptor.
        let result = unsafe { SetEntriesInAclW(1, &entry, acl, &mut modified) };
        ensure!(
            result == 0,
            "cannot build cursor permissions: Windows error {result}"
        );
        let _modified = Local(modified.cast());
        // SAFETY: modified is a valid owned ACL. Owner, SACL and all other
        // principals' entries remain unchanged; only this random SID is edited.
        let result = unsafe {
            SetSecurityInfo(
                self.station,
                SE_WINDOW_OBJECT,
                DACL_SECURITY_INFORMATION,
                null_mut(),
                null_mut(),
                modified,
                null(),
            )
        };
        ensure!(
            result == 0,
            "cannot apply cursor permissions: Windows error {result}"
        );
        Ok(())
    }
}
impl Drop for Grant {
    fn drop(&mut self) {
        if self.registered
            && let Err(error) = self.update(REVOKE_ACCESS)
        {
            eprintln!("Warning: cannot remove expired cursor capability: {error}");
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use windows_sys::Win32::{
        Security::{ACE_HEADER, GetAce},
        System::StationsAndDesktops::{CloseWindowStation, CreateWindowStationW},
    };

    struct Station(HANDLE);
    impl Drop for Station {
        fn drop(&mut self) {
            // SAFETY: uniquely owns a private station, not the process station.
            unsafe {
                CloseWindowStation(self.0);
            }
        }
    }

    fn entries(station: HANDLE) -> Result<Vec<Vec<u8>>> {
        let mut acl = null_mut();
        let mut descriptor = null_mut();
        // SAFETY: valid station and SDK output pointers.
        let result = unsafe {
            GetSecurityInfo(
                station,
                SE_WINDOW_OBJECT,
                DACL_SECURITY_INFORMATION,
                null_mut(),
                null_mut(),
                &mut acl,
                null_mut(),
                &mut descriptor,
            )
        };
        ensure!(result == 0, "cannot read private station ACL: {result}");
        let _descriptor = Local(descriptor);
        ensure!(!acl.is_null(), "missing test DACL");
        let mut entries = Vec::new();
        // SAFETY: the live security descriptor owns the ACL and all its ACEs.
        for index in 0..unsafe { (*acl).AceCount } {
            let mut ace = null_mut();
            check(unsafe { GetAce(acl, index as u32, &mut ace) })?;
            let size = unsafe { (*ace.cast::<ACE_HEADER>()).AceSize };
            entries.push(
                unsafe { std::slice::from_raw_parts(ace.cast::<u8>(), size as usize) }.to_vec(),
            );
        }
        entries.sort();
        Ok(entries)
    }

    #[test]
    fn overlapping_launch_grants_are_scoped_and_removed_independently() -> Result<()> {
        // SAFETY: creates an unselected, private station with an OS-chosen name.
        // READ_CONTROL | WRITE_DAC are sufficient for this ACL-only test.
        let station = Station(unsafe { CreateWindowStationW(null(), 0, 0x00060000, null()) });
        ensure!(
            !station.0.is_null(),
            "cannot create private station: {}",
            io::Error::last_os_error()
        );
        let original = entries(station.0)?;
        let first = Grant::for_station(station.0)?;
        let after_first = entries(station.0)?;
        let first_ace: Vec<_> = after_first
            .iter()
            .filter(|ace| !original.contains(ace))
            .collect();
        ensure!(first_ace.len() == 1, "expected one new cursor ACE");
        ensure!(
            first_ace[0][0] == 0 && first_ace[0][1] == 0,
            "expected non-inherited allow ACE"
        );
        ensure!(
            u32::from_le_bytes(first_ace[0][4..8].try_into()?) == WINSTA_WRITEATTRIBUTES as u32,
            "cursor grant contains unrelated rights"
        );
        let second = Grant::for_station(station.0)?;
        ensure!(
            entries(station.0)?.len() == original.len() + 2,
            "capabilities must be unique per launch"
        );
        drop(first);
        let remaining = entries(station.0)?;
        ensure!(
            remaining.len() == original.len() + 1
                && original.iter().all(|ace| remaining.contains(ace)),
            "revocation lost another principal's grant"
        );
        drop(second);
        ensure!(
            entries(station.0)? == original,
            "cursor ACE was not removed"
        );
        Ok(())
    }
}
