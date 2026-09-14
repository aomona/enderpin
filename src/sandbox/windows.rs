//! AppContainer identity + exact filesystem grants + kill-on-close process job.
//! Windows SDK owns enforcement; failures never launch with an ordinary token.
#![allow(unsafe_code)]
use super::Policy;
use anyhow::{Context, Result, ensure};
use std::{
    ffi::{OsStr, c_void},
    fs::{self, File},
    io,
    mem::size_of,
    os::windows::{
        ffi::OsStrExt,
        io::{AsRawHandle, FromRawHandle, OwnedHandle},
        process::{CommandExt, ExitStatusExt},
    },
    path::{Path, PathBuf},
    process::{Command, ExitStatus},
    ptr::{null, null_mut},
};
use windows_sys::Win32::{
    Foundation::{
        ERROR_ALREADY_EXISTS, HANDLE, HANDLE_FLAG_INHERIT, LocalFree, SetHandleInformation,
        WAIT_OBJECT_0,
    },
    Security::{
        Authorization::{
            ConvertSidToStringSidW, ConvertStringSidToSidW, EXPLICIT_ACCESS_W,
            GetNamedSecurityInfoW, SE_FILE_OBJECT, SET_ACCESS, SetEntriesInAclW, TRUSTEE_IS_SID,
            TRUSTEE_IS_UNKNOWN, TRUSTEE_W,
        },
        DACL_SECURITY_INFORMATION, FreeSid, GetSecurityDescriptorControl,
        InitializeSecurityDescriptor,
        Isolation::{CreateAppContainerProfile, DeriveAppContainerSidFromAppContainerName},
        SE_DACL_AUTO_INHERIT_REQ, SE_DACL_AUTO_INHERITED, SE_DACL_PROTECTED, SECURITY_ATTRIBUTES,
        SECURITY_CAPABILITIES, SECURITY_DESCRIPTOR, SID_AND_ATTRIBUTES, SetFileSecurityW,
        SetSecurityDescriptorControl, SetSecurityDescriptorDacl,
    },
    System::{
        JobObjects::{
            AssignProcessToJobObject, CreateJobObjectW, JOB_OBJECT_LIMIT_KILL_ON_JOB_CLOSE,
            JOBOBJECT_EXTENDED_LIMIT_INFORMATION, JobObjectExtendedLimitInformation,
            SetInformationJobObject, TerminateJobObject,
        },
        Pipes::CreatePipe,
        SystemInformation::GetSystemDirectoryW,
        Threading::{
            CREATE_NO_WINDOW, CREATE_SUSPENDED, CREATE_UNICODE_ENVIRONMENT, CreateProcessW,
            DeleteProcThreadAttributeList, EXTENDED_STARTUPINFO_PRESENT, GetExitCodeProcess,
            InitializeProcThreadAttributeList, PROC_THREAD_ATTRIBUTE_HANDLE_LIST,
            PROC_THREAD_ATTRIBUTE_SECURITY_CAPABILITIES, PROCESS_INFORMATION, ResumeThread,
            STARTF_USESTDHANDLES, STARTUPINFOEXW, TerminateProcess, UpdateProcThreadAttribute,
            WaitForSingleObject,
        },
    },
};

fn wide(value: &OsStr) -> Result<Vec<u16>> {
    let mut value: Vec<_> = value.encode_wide().collect();
    ensure!(!value.contains(&0), "Windows string contains NUL");
    value.push(0);
    Ok(value)
}
fn check(ok: i32) -> io::Result<()> {
    if ok == 0 {
        Err(io::Error::last_os_error())
    } else {
        Ok(())
    }
}
fn owned(handle: HANDLE) -> io::Result<OwnedHandle> {
    if handle.is_null() || handle == -1isize as HANDLE {
        return Err(io::Error::last_os_error());
    }
    // SAFETY: caller transfers a newly created, unique, valid kernel handle.
    Ok(unsafe { OwnedHandle::from_raw_handle(handle) })
}
fn system_directory() -> Result<PathBuf> {
    let mut text = vec![0u16; 32768];
    // SAFETY: buffer and reported capacity agree; API writes at most its capacity.
    let count = unsafe { GetSystemDirectoryW(text.as_mut_ptr(), text.len() as u32) } as usize;
    ensure!(
        count > 0 && count < text.len(),
        "cannot locate Windows system directory"
    );
    Ok(PathBuf::from(String::from_utf16(&text[..count])?))
}

/// Configuration only; callers must use this module's spawn, never Command::spawn.
pub(crate) fn configuration(java: &Path, policy: &Policy) -> Result<Command> {
    policy.validate(java)?;
    let system = system_directory()?;
    let mut command = Command::new(crate::storage::java_path(java));
    command.env_clear();
    super::environment(&mut command, policy);
    command
        .env(
            "SystemRoot",
            system.parent().context("invalid system directory")?,
        )
        .env(
            "WINDIR",
            system.parent().context("invalid system directory")?,
        )
        .env("APPDATA", crate::storage::java_path(&policy.game))
        .env("LOCALAPPDATA", crate::storage::java_path(&policy.temporary));
    Ok(command)
}

struct Sid(*mut c_void);
impl Drop for Sid {
    fn drop(&mut self) {
        /* SAFETY: SDK-allocated SID, owned once. */
        unsafe {
            FreeSid(self.0);
        }
    }
}
struct Local(*mut c_void);
impl Drop for Local {
    fn drop(&mut self) {
        /* SAFETY: LocalAlloc-compatible SDK output, owned once. */
        unsafe {
            LocalFree(self.0);
        }
    }
}

fn identity(policy: &Policy) -> Result<(Sid, String)> {
    // Changed grants get a different token identity: old host ACLs cannot retain
    // authority after a local permission is revoked or a folder binding changes.
    let identity = serde_json::to_vec(&(
        &policy.game,
        &policy.java_home,
        &policy.readonly,
        &policy.extra,
        &policy.permissions,
        policy.network,
        policy.desktop,
    ))?;
    let name = format!("Enderpin.{}", &crate::model::hash_bytes(&identity)[..40]);
    let name = wide(OsStr::new(&name))?;
    let mut sid = null_mut();
    // SAFETY: terminated name and valid output pointer; no capabilities at profile creation.
    let result = unsafe {
        CreateAppContainerProfile(
            name.as_ptr(),
            name.as_ptr(),
            name.as_ptr(),
            null(),
            0,
            &mut sid,
        )
    };
    if result < 0 {
        ensure!(
            result as u32 == 0x80070000 | ERROR_ALREADY_EXISTS,
            "AppContainer profile creation failed: {result:#x}"
        );
        // SAFETY: same validated name; successful call allocates an owned SID.
        let result = unsafe { DeriveAppContainerSidFromAppContainerName(name.as_ptr(), &mut sid) };
        ensure!(
            result >= 0,
            "AppContainer identity lookup failed: {result:#x}"
        );
    }
    ensure!(!sid.is_null(), "AppContainer returned no SID");
    let sid = Sid(sid);
    let mut text = null_mut();
    // SAFETY: sid is a valid owned SID; SDK allocates a terminated UTF-16 string.
    check(unsafe { ConvertSidToStringSidW(sid.0, &mut text) })?;
    let allocation = Local(text.cast());
    let mut len = 0;
    // SAFETY: ConvertSidToStringSidW guarantees a terminated string, shorter than 184 chars.
    while len < 184 && unsafe { *text.add(len) } != 0 {
        len += 1;
    }
    ensure!(len < 184, "unexpected SID string length");
    let string = String::from_utf16(unsafe { std::slice::from_raw_parts(text, len) })?;
    drop(allocation);
    Ok((sid, string))
}

pub(super) fn validate_single_link(path: &Path) -> Result<()> {
    use windows_sys::Win32::Storage::FileSystem::{
        BY_HANDLE_FILE_INFORMATION, GetFileInformationByHandle,
    };
    let file = File::open(path)?;
    let mut info: BY_HANDLE_FILE_INFORMATION = unsafe { std::mem::zeroed() };
    // SAFETY: file owns the handle and info is a valid output buffer.
    check(unsafe { GetFileInformationByHandle(file.as_raw_handle(), &mut info) })?;
    ensure!(
        info.nNumberOfLinks == 1,
        "sandbox data contains a hard link: {}",
        path.display()
    );
    Ok(())
}

fn validate_tree(root: &Path) -> Result<()> {
    let mut paths = vec![root.to_owned()];
    let mut count = 0;
    while let Some(path) = paths.pop() {
        count += 1;
        ensure!(count <= 1_000_000, "sandbox tree is too large");
        let metadata = fs::symlink_metadata(&path)?;
        ensure!(
            !crate::storage::is_link(&metadata),
            "sandbox tree contains a reparse point: {}",
            path.display()
        );
        if metadata.is_dir() {
            for entry in fs::read_dir(path)? {
                paths.push(entry?.path());
            }
        }
    }
    Ok(())
}
fn icacls(path: &Path, args: &[String]) -> Result<()> {
    let system = system_directory()?;
    let output = Command::new(system.join("icacls.exe"))
        .arg(path)
        .args(args)
        .env_clear()
        .env(
            "SystemRoot",
            system.parent().context("invalid system directory")?,
        )
        .creation_flags(CREATE_NO_WINDOW)
        .output()?;
    ensure!(
        output.status.success(),
        "cannot apply AppContainer access to {} (icacls {})",
        path.display(),
        output.status
    );
    Ok(())
}

fn grant_traverse(path: &Path, sid: &Sid) -> Result<()> {
    let path = wide(path.as_os_str())?;
    let (mut old_acl, mut descriptor) = (null_mut(), null_mut());
    // SAFETY: SDK initializes the output pointers; descriptor owns old_acl's storage.
    let error = unsafe {
        GetNamedSecurityInfoW(
            path.as_ptr(),
            SE_FILE_OBJECT,
            DACL_SECURITY_INFORMATION,
            null_mut(),
            null_mut(),
            &mut old_acl,
            null_mut(),
            &mut descriptor,
        )
    };
    if error != 0 {
        return Err(io::Error::from_raw_os_error(error as i32).into());
    }
    let descriptor = Local(descriptor);
    // A null DACL already grants everyone access. Do not accidentally restrict it.
    if old_acl.is_null() {
        return Ok(());
    }
    let entry = EXPLICIT_ACCESS_W {
        // FILE_TRAVERSE | FILE_READ_ATTRIBUTES | READ_CONTROL | SYNCHRONIZE;
        // no listing, file read, write or inheritance permission.
        grfAccessPermissions: 0x20 | 0x80 | 0x20000 | 0x100000,
        // Replace this SID's previous ancestor ACE, including inheritance flags.
        grfAccessMode: SET_ACCESS,
        grfInheritance: 0,
        Trustee: TRUSTEE_W {
            pMultipleTrustee: null_mut(),
            MultipleTrusteeOperation: 0,
            TrusteeForm: TRUSTEE_IS_SID,
            TrusteeType: TRUSTEE_IS_UNKNOWN,
            ptstrName: sid.0.cast(),
        },
    };
    let mut acl = null_mut();
    // SAFETY: entry, SID and original descriptor remain alive for the merge.
    let error = unsafe { SetEntriesInAclW(1, &entry, old_acl, &mut acl) };
    if error != 0 {
        return Err(io::Error::from_raw_os_error(error as i32).into());
    }
    let _acl = Local(acl.cast());
    let (mut control, mut revision) = (0, 0);
    let mut updated: SECURITY_DESCRIPTOR = unsafe { std::mem::zeroed() };
    let updated_ptr = (&mut updated as *mut SECURITY_DESCRIPTOR).cast();
    // SAFETY: all SDK buffers and ACL allocations remain valid through the update.
    unsafe {
        check(GetSecurityDescriptorControl(
            descriptor.0,
            &mut control,
            &mut revision,
        ))?;
        check(InitializeSecurityDescriptor(updated_ptr, 1))?;
        check(SetSecurityDescriptorDacl(updated_ptr, 1, acl, 0))?;
        let mask = SE_DACL_PROTECTED | SE_DACL_AUTO_INHERITED | SE_DACL_AUTO_INHERIT_REQ;
        check(SetSecurityDescriptorControl(
            updated_ptr,
            mask,
            control & mask,
        ))?;
        // Intentionally use SetFileSecurity: unlike SetNamedSecurityInfo/icacls,
        // it updates only this directory, without reapplying ACLs to every child.
        check(SetFileSecurityW(
            path.as_ptr(),
            DACL_SECURITY_INFORMATION,
            updated_ptr,
        ))?;
    }
    Ok(())
}

fn grant_tree(path: &Path, sid: &str, rights: &str) -> Result<()> {
    // An inheritance-only grant can miss files whose inheritance was disabled.
    // Apply direct rights to every existing object, then add future-child rights.
    for (mode, flags) in [("/grant:r", ""), ("/grant", "(OI)(CI)")] {
        icacls(
            path,
            &[
                mode.into(),
                format!("*{sid}:{flags}{rights}"),
                "/T".into(),
                "/Q".into(),
            ],
        )?;
    }
    Ok(())
}

fn grants(policy: &Policy, sid: &Sid, sid_text: &str) -> Result<()> {
    // Only this target's SID is changed. Never grant ALL APPLICATION PACKAGES or
    // recurse through game-controlled junctions into the host's files.
    let grants = policy.grants();
    for grant in &grants {
        let (path, writable) = (&grant.path, grant.write);
        validate_tree(path)?;
        for ancestor in path.ancestors().skip(1).filter(|p| p.parent().is_some()) {
            // Managed ancestors receive their declared RX/M tree grant; do not
            // narrow them to traversal when preparing nested Java/tmp roots.
            if grants.iter().any(|grant| ancestor.starts_with(&grant.path)) {
                continue;
            }
            grant_traverse(ancestor, sid)?;
        }
        grant_tree(path, sid_text, if writable { "M" } else { "RX" })?;
        if writable {
            icacls(
                path,
                &[
                    "/setintegritylevel".into(),
                    "(OI)(CI)L".into(),
                    "/T".into(),
                    "/Q".into(),
                ],
            )?;
        }
    }
    for path in policy.read_only_game()? {
        // Protect the boundary, preserving other principals' inherited entries.
        // A parent Modify grant has no DELETE_CHILD; RX prevents replacing this
        // subtree through rename/delete as well as writing its files.
        icacls(
            &path,
            &["/inheritancelevel:d".into(), "/T".into(), "/Q".into()],
        )?;
        icacls(
            &path,
            &[
                "/remove:g".into(),
                format!("*{sid_text}"),
                "/T".into(),
                "/Q".into(),
            ],
        )?;
        grant_tree(&path, sid_text, "RX")?;
    }
    Ok(())
}

fn pipe(child_reads: bool) -> Result<(OwnedHandle, File)> {
    let attrs = SECURITY_ATTRIBUTES {
        nLength: size_of::<SECURITY_ATTRIBUTES>() as u32,
        lpSecurityDescriptor: null_mut(),
        bInheritHandle: 1,
    };
    let (mut read, mut write) = (null_mut(), null_mut());
    // SAFETY: attributes and both output pointers remain valid throughout the call.
    check(unsafe { CreatePipe(&mut read, &mut write, &attrs, 0) })?;
    let read = owned(read)?;
    let write = owned(write)?;
    let (child, parent) = if child_reads {
        (read, write)
    } else {
        (write, read)
    };
    check(unsafe { SetHandleInformation(parent.as_raw_handle(), HANDLE_FLAG_INHERIT, 0) })?;
    Ok((child, parent.into()))
}

pub struct Process {
    process: OwnedHandle,
    job: OwnedHandle,
    id: u32,
    pub stdin: Option<File>,
    pub(crate) stdout: Option<File>,
    pub(crate) stderr: Option<File>,
}
impl Process {
    pub fn id(&self) -> u32 {
        self.id
    }
    pub fn try_wait(&mut self) -> io::Result<Option<ExitStatus>> {
        // SAFETY: process handle is retained for the lifetime of self.
        let result = unsafe { WaitForSingleObject(self.process.as_raw_handle(), 0) };
        if result == 258 {
            return Ok(None);
        }
        if result != WAIT_OBJECT_0 {
            return Err(io::Error::last_os_error());
        }
        let mut code = 0;
        check(unsafe { GetExitCodeProcess(self.process.as_raw_handle(), &mut code) })?;
        Ok(Some(ExitStatus::from_raw(code)))
    }
    pub fn wait(&mut self) -> io::Result<ExitStatus> {
        if unsafe { WaitForSingleObject(self.process.as_raw_handle(), u32::MAX) } != WAIT_OBJECT_0 {
            return Err(io::Error::last_os_error());
        }
        self.try_wait()?
            .ok_or_else(|| io::Error::other("process has not exited"))
    }
    pub fn kill(&mut self) -> io::Result<()> {
        check(unsafe { TerminateJobObject(self.job.as_raw_handle(), 1) })
    }
}
impl Drop for Process {
    fn drop(&mut self) {
        let _ = self.kill();
        let _ = self.wait();
    }
}

struct Attributes {
    pointer: *mut c_void,
    _storage: Vec<usize>,
}
impl Attributes {
    fn new() -> Result<Self> {
        let mut bytes = 0;
        // SAFETY: first call queries required allocation size.
        unsafe {
            InitializeProcThreadAttributeList(null_mut(), 2, 0, &mut bytes);
        }
        ensure!(
            bytes > 0 && bytes < 65536,
            "invalid Windows attribute list size"
        );
        let mut storage = vec![0usize; bytes.div_ceil(size_of::<usize>())];
        let pointer = storage.as_mut_ptr().cast();
        check(unsafe { InitializeProcThreadAttributeList(pointer, 2, 0, &mut bytes) })?;
        Ok(Self {
            pointer,
            _storage: storage,
        })
    }
}
impl Drop for Attributes {
    fn drop(&mut self) {
        unsafe {
            DeleteProcThreadAttributeList(self.pointer);
        }
    }
}

pub(crate) fn spawn_inner(
    command: &Command,
    policy: &Policy,
    inherited: &[std::os::windows::io::BorrowedHandle<'_>],
) -> Result<Process> {
    policy.validate(Path::new(command.get_program()))?;
    let (sid, sid_text) = identity(policy)?;
    grants(policy, &sid, &sid_text)?;
    let mut allocations = vec![];
    let mut capabilities = vec![];
    if policy.network {
        for value in ["S-1-15-3-1", "S-1-15-3-2", "S-1-15-3-3"] {
            let value = wide(OsStr::new(value))?;
            let mut pointer = null_mut();
            check(unsafe { ConvertStringSidToSidW(value.as_ptr(), &mut pointer) })?;
            allocations.push(Local(pointer));
            capabilities.push(SID_AND_ATTRIBUTES {
                Sid: pointer,
                Attributes: 4,
            });
        }
    }
    let security = SECURITY_CAPABILITIES {
        AppContainerSid: sid.0,
        Capabilities: if capabilities.is_empty() {
            null_mut()
        } else {
            capabilities.as_mut_ptr()
        },
        CapabilityCount: capabilities.len() as u32,
        Reserved: 0,
    };
    let (input_child, input) = pipe(true)?;
    let (output_child, output) = pipe(false)?;
    let (error_child, error) = pipe(false)?;
    let mut handles = vec![
        input_child.as_raw_handle(),
        output_child.as_raw_handle(),
        error_child.as_raw_handle(),
    ];
    handles.extend(inherited.iter().map(|handle| handle.as_raw_handle()));
    let attributes = Attributes::new()?;
    // SAFETY: payloads and inherited handles remain alive until CreateProcessW returns.
    check(unsafe {
        UpdateProcThreadAttribute(
            attributes.pointer,
            0,
            PROC_THREAD_ATTRIBUTE_SECURITY_CAPABILITIES as usize,
            (&security as *const SECURITY_CAPABILITIES).cast(),
            size_of::<SECURITY_CAPABILITIES>(),
            null_mut(),
            null(),
        )
    })?;
    check(unsafe {
        UpdateProcThreadAttribute(
            attributes.pointer,
            0,
            PROC_THREAD_ATTRIBUTE_HANDLE_LIST as usize,
            handles.as_ptr().cast(),
            size_of_val(handles.as_slice()),
            null_mut(),
            null(),
        )
    })?;
    let job = owned(unsafe { CreateJobObjectW(null(), null()) })?;
    let mut limits: JOBOBJECT_EXTENDED_LIMIT_INFORMATION = unsafe { std::mem::zeroed() };
    limits.BasicLimitInformation.LimitFlags = JOB_OBJECT_LIMIT_KILL_ON_JOB_CLOSE;
    check(unsafe {
        SetInformationJobObject(
            job.as_raw_handle(),
            JobObjectExtendedLimitInformation,
            (&limits as *const JOBOBJECT_EXTENDED_LIMIT_INFORMATION).cast(),
            size_of_val(&limits) as u32,
        )
    })?;
    let application = wide(command.get_program())?;
    let mut line = String::new();
    for value in std::iter::once(command.get_program()).chain(command.get_args()) {
        if !line.is_empty() {
            line.push(' ');
        }
        line.push_str(&quote(
            value.to_str().context("Windows arguments must be UTF-8")?,
        ));
    }
    let mut line = wide(OsStr::new(&line))?;
    ensure!(line.len() <= 32767, "Windows command line is too long");
    let cwd = wide(crate::storage::java_path(&policy.game).as_os_str())?;
    let mut envs: Vec<_> = command
        .get_envs()
        .filter_map(|(k, v)| v.map(|v| (k, v)))
        .collect();
    envs.sort_by_key(|(k, _)| k.to_string_lossy().to_uppercase());
    let mut environment = vec![];
    for (key, value) in envs {
        ensure!(
            !key.to_string_lossy().contains('='),
            "invalid Windows environment key"
        );
        let entry = format!(
            "{}={}",
            key.to_str().context("non-UTF8 environment key")?,
            value.to_str().context("non-UTF8 environment value")?
        );
        environment.extend(wide(OsStr::new(&entry))?);
    }
    environment.push(0u16);
    let mut startup: STARTUPINFOEXW = unsafe { std::mem::zeroed() };
    startup.StartupInfo.cb = size_of::<STARTUPINFOEXW>() as u32;
    startup.StartupInfo.dwFlags = STARTF_USESTDHANDLES;
    startup.StartupInfo.hStdInput = handles[0];
    startup.StartupInfo.hStdOutput = handles[1];
    startup.StartupInfo.hStdError = handles[2];
    startup.lpAttributeList = attributes.pointer;
    let mut info: PROCESS_INFORMATION = unsafe { std::mem::zeroed() };
    // SAFETY: every pointer references a live buffer; JVM starts suspended so it
    // cannot execute before assignment to the non-breakaway kill-on-close job.
    check(unsafe {
        CreateProcessW(
            application.as_ptr(),
            line.as_mut_ptr(),
            null(),
            null(),
            1,
            EXTENDED_STARTUPINFO_PRESENT
                | CREATE_SUSPENDED
                | CREATE_UNICODE_ENVIRONMENT
                | CREATE_NO_WINDOW,
            environment.as_ptr().cast(),
            cwd.as_ptr(),
            &startup.StartupInfo,
            &mut info,
        )
    })?;
    let process = owned(info.hProcess)?;
    let thread = owned(info.hThread)?;
    if let Err(error) =
        check(unsafe { AssignProcessToJobObject(job.as_raw_handle(), process.as_raw_handle()) })
    {
        unsafe {
            TerminateProcess(process.as_raw_handle(), 1);
            WaitForSingleObject(process.as_raw_handle(), u32::MAX);
        }
        return Err(error.into());
    }
    if unsafe { ResumeThread(thread.as_raw_handle()) } == u32::MAX {
        unsafe {
            TerminateJobObject(job.as_raw_handle(), 1);
            WaitForSingleObject(process.as_raw_handle(), u32::MAX);
        }
        return Err(io::Error::last_os_error().into());
    }
    Ok(Process {
        process,
        job,
        id: info.dwProcessId,
        stdin: Some(input),
        stdout: Some(output),
        stderr: Some(error),
    })
}

fn quote(value: &str) -> String {
    let mut output = String::from("\"");
    let mut slashes = 0;
    for ch in value.chars() {
        if ch == '\\' {
            slashes += 1;
            continue;
        }
        output.extend(std::iter::repeat_n(
            '\\',
            if ch == '"' { slashes * 2 + 1 } else { slashes },
        ));
        output.push(ch);
        slashes = 0;
    }
    output.extend(std::iter::repeat_n('\\', slashes * 2));
    output.push('"');
    output
}

/// Captured process output for enforcement tests. Same native launch path as games.
pub fn output(
    java: &Path,
    policy: &Policy,
    args: &[std::ffi::OsString],
) -> Result<std::process::Output> {
    output_inherited(java, policy, args, &[])
}

pub(crate) fn output_inherited(
    java: &Path,
    policy: &Policy,
    arguments: &[std::ffi::OsString],
    inherited: &[std::os::windows::io::BorrowedHandle<'_>],
) -> Result<std::process::Output> {
    let mut command = configuration(java, policy)?;
    command.args(arguments);
    eprintln!("AppContainer probe: preparing process");
    let mut process = spawn_inner(&command, policy, inherited)?;
    eprintln!("AppContainer probe: process {} started", process.id());
    process.stdin.take();
    let mut stdout = process.stdout.take().context("stdout missing")?;
    let mut stderr = process.stderr.take().context("stderr missing")?;
    let out = std::thread::spawn(move || {
        use io::Read;
        let mut bytes = vec![];
        stdout.read_to_end(&mut bytes).map(|_| bytes)
    });
    let err = std::thread::spawn(move || {
        use io::Read;
        let mut bytes = vec![];
        stderr.read_to_end(&mut bytes).map(|_| bytes)
    });
    let started = std::time::Instant::now();
    let mut timed_out = false;
    let status = loop {
        if let Some(status) = process.try_wait()? {
            break status;
        }
        if started.elapsed() > std::time::Duration::from_secs(60) {
            timed_out = true;
            process.kill()?;
            break process.wait()?;
        }
        std::thread::sleep(std::time::Duration::from_millis(100));
    };
    eprintln!("AppContainer probe: process exited with {status}; collecting output");
    let output = std::process::Output {
        status,
        stdout: out
            .join()
            .map_err(|_| anyhow::anyhow!("stdout worker failed"))??,
        stderr: err
            .join()
            .map_err(|_| anyhow::anyhow!("stderr worker failed"))??,
    };
    ensure!(
        !timed_out,
        "sandbox probe timed out:\n{}\n{}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
    Ok(output)
}
