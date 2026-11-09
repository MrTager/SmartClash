//! 生命周期所需的 Windows 句柄、身份、私有同步对象与 Job。
use serde::{Deserialize, Serialize};
use std::os::windows::{ffi::OsStrExt, io::AsRawHandle, process::CommandExt};
use std::path::{Path, PathBuf};
use std::process::{Child, Command};
use windows_sys::Win32::{
    Foundation::*,
    Security::{Authorization::*, *},
    Storage::FileSystem::*,
    System::{JobObjects::*, Threading::*},
};

pub fn wide(value: impl AsRef<std::ffi::OsStr>) -> Vec<u16> {
    value.as_ref().encode_wide().chain(Some(0)).collect()
}
pub fn error(context: &str) -> String {
    format!("{context}：{}", std::io::Error::last_os_error())
}

pub struct Handle(pub HANDLE);
unsafe impl Send for Handle {}
unsafe impl Sync for Handle {}
impl Drop for Handle {
    fn drop(&mut self) {
        unsafe {
            CloseHandle(self.0);
        }
    }
}
impl Handle {
    pub fn signaled(&self) -> Result<bool, String> {
        match unsafe { WaitForSingleObject(self.0, 0) } {
            WAIT_OBJECT_0 => Ok(true),
            WAIT_TIMEOUT => Ok(false),
            _ => Err(error("查询句柄失败")),
        }
    }
    pub fn wait(&self, ms: u32) -> Result<bool, String> {
        match unsafe { WaitForSingleObject(self.0, ms) } {
            WAIT_OBJECT_0 => Ok(true),
            WAIT_TIMEOUT => Ok(false),
            _ => Err(error("等待句柄失败")),
        }
    }
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct Identity {
    pub pid: u32,
    pub created: u64,
    pub exe: PathBuf,
    pub sid: String,
}

pub fn same_path(a: &Path, b: &Path) -> bool {
    fn norm(p: &Path) -> String {
        p.to_string_lossy()
            .trim_start_matches(r"\\?\")
            .replace('/', "\\")
            .to_lowercase()
    }
    norm(a) == norm(b)
}
fn token_sid(process: HANDLE) -> Result<String, String> {
    unsafe {
        let mut raw = std::ptr::null_mut();
        if OpenProcessToken(process, TOKEN_QUERY, &mut raw) == 0 {
            return Err(error("读取进程令牌失败"));
        }
        let token = Handle(raw);
        let mut needed = 0;
        GetTokenInformation(token.0, TokenUser, std::ptr::null_mut(), 0, &mut needed);
        let mut buffer = vec![0usize; (needed as usize + 7) / 8];
        if GetTokenInformation(
            token.0,
            TokenUser,
            buffer.as_mut_ptr().cast(),
            needed,
            &mut needed,
        ) == 0
        {
            return Err(error("读取用户 SID 失败"));
        }
        let user = &*(buffer.as_ptr() as *const TOKEN_USER);
        let mut text = std::ptr::null_mut();
        if ConvertSidToStringSidW(user.User.Sid, &mut text) == 0 {
            return Err(error("转换 SID 失败"));
        }
        let mut len = 0;
        while *text.add(len) != 0 {
            len += 1;
        }
        let result = String::from_utf16_lossy(std::slice::from_raw_parts(text, len));
        LocalFree(text.cast());
        Ok(result)
    }
}
pub fn user_sid() -> Result<String, String> {
    token_sid(unsafe { GetCurrentProcess() })
}
pub fn identity_handle(handle: HANDLE, pid: u32) -> Result<Identity, String> {
    unsafe {
        let mut created = std::mem::zeroed();
        let mut exit = std::mem::zeroed();
        let mut kernel = std::mem::zeroed();
        let mut user = std::mem::zeroed();
        if GetProcessTimes(handle, &mut created, &mut exit, &mut kernel, &mut user) == 0 {
            return Err(error("读取进程创建时间失败"));
        }
        let mut buffer = vec![0u16; 32768];
        let mut len = buffer.len() as u32;
        if QueryFullProcessImageNameW(handle, 0, buffer.as_mut_ptr(), &mut len) == 0 {
            return Err(error("读取进程路径失败"));
        }
        Ok(Identity {
            pid,
            created: ((created.dwHighDateTime as u64) << 32) | created.dwLowDateTime as u64,
            exe: PathBuf::from(String::from_utf16_lossy(&buffer[..len as usize])),
            sid: token_sid(handle)?,
        })
    }
}
pub fn current_identity() -> Result<Identity, String> {
    identity_handle(unsafe { GetCurrentProcess() }, std::process::id())
}
pub fn open_identity(expected: &Identity) -> Result<Option<Handle>, String> {
    let raw = unsafe {
        OpenProcess(
            PROCESS_QUERY_LIMITED_INFORMATION | SYNCHRONIZE,
            0,
            expected.pid,
        )
    };
    if raw.is_null() {
        if unsafe { GetLastError() } == ERROR_INVALID_PARAMETER {
            return Ok(None);
        }
        return Err(error("无法确认会话进程身份，保留记录"));
    }
    let handle = Handle(raw);
    if handle.signaled()? {
        return Ok(None);
    }
    let actual = identity_handle(handle.0, expected.pid)?;
    if actual.created != expected.created
        || !same_path(&actual.exe, &expected.exe)
        || actual.sid != expected.sid
    {
        return Err("进程身份不符（可能 PID 复用），保留记录等待人工恢复".into());
    }
    if handle.signaled()? {
        Ok(None)
    } else {
        Ok(Some(handle))
    }
}
pub fn inspect(pid: u32) -> Result<Option<Identity>, String> {
    let raw = unsafe { OpenProcess(PROCESS_QUERY_LIMITED_INFORMATION | SYNCHRONIZE, 0, pid) };
    if raw.is_null() {
        if unsafe { GetLastError() } == ERROR_INVALID_PARAMETER {
            return Ok(None);
        }
        return Err(error("无法查询遗留 PID"));
    }
    let h = Handle(raw);
    if h.signaled()? {
        return Ok(None);
    }
    identity_handle(h.0, pid).map(Some)
}

struct Security {
    descriptor: PSECURITY_DESCRIPTOR,
    attributes: SECURITY_ATTRIBUTES,
}
impl Security {
    fn ipc(sid: &str) -> Result<Self, String> {
        Self::sddl(&format!("D:P(A;;GA;;;SY)(A;;GA;;;{sid})S:(ML;;NW;;;ME)"))
    }
    fn sddl(sddl: &str) -> Result<Self, String> {
        let sddl = wide(sddl);
        let mut descriptor = std::ptr::null_mut();
        if unsafe {
            ConvertStringSecurityDescriptorToSecurityDescriptorW(
                sddl.as_ptr(),
                1,
                &mut descriptor,
                std::ptr::null_mut(),
            )
        } == 0
        {
            return Err(error("创建私有 ACL 失败"));
        }
        Ok(Self {
            descriptor,
            attributes: SECURITY_ATTRIBUTES {
                nLength: std::mem::size_of::<SECURITY_ATTRIBUTES>() as u32,
                lpSecurityDescriptor: descriptor,
                bInheritHandle: 0,
            },
        })
    }
}
impl Drop for Security {
    fn drop(&mut self) {
        unsafe {
            LocalFree(self.descriptor);
        }
    }
}
pub fn elevated() -> Result<bool, String> {
    unsafe {
        let mut raw = std::ptr::null_mut();
        if OpenProcessToken(GetCurrentProcess(), TOKEN_QUERY, &mut raw) == 0 {
            return Err(error("无法读取提权令牌"));
        }
        let token = Handle(raw);
        let mut info: TOKEN_ELEVATION = std::mem::zeroed();
        let mut len = 0;
        if GetTokenInformation(
            token.0,
            TokenElevation,
            (&mut info as *mut TOKEN_ELEVATION).cast(),
            std::mem::size_of_val(&info) as u32,
            &mut len,
        ) == 0
        {
            return Err(error("无法确认提权状态"));
        }
        Ok(info.TokenIsElevated != 0)
    }
}
fn admin_security(owner_sid: &str) -> Result<Security, String> {
    Security::sddl(&format!(
        "O:BAD:P(A;OICI;GA;;;SY)(A;OICI;GA;;;BA)(A;OICI;RC;;;OW)(A;OICI;GRGX;;;{owner_sid})"
    ))
}
pub fn create_admin_directory(path: &Path, owner_sid: &str) -> Result<(), String> {
    let sec = admin_security(owner_sid)?;
    if unsafe { CreateDirectoryW(wide(path).as_ptr(), &sec.attributes) } == 0 {
        return Err(error("原子创建管理员工作目录失败"));
    }
    Ok(())
}
pub fn create_admin_file(path: &Path, owner_sid: &str) -> Result<std::fs::File, String> {
    use std::os::windows::io::FromRawHandle;
    let sec = admin_security(owner_sid)?;
    let raw = unsafe {
        CreateFileW(
            wide(path).as_ptr(),
            GENERIC_WRITE,
            FILE_SHARE_READ,
            &sec.attributes,
            CREATE_NEW,
            FILE_ATTRIBUTE_NORMAL,
            std::ptr::null_mut(),
        )
    };
    if raw == INVALID_HANDLE_VALUE {
        return Err(error("原子创建管理员受控文件失败"));
    }
    Ok(unsafe { std::fs::File::from_raw_handle(raw.cast()) })
}
fn user_file_security(sid: &str) -> Result<Security, String> {
    Security::sddl(&format!(
        "O:{sid}D:P(A;;GA;;;SY)(A;;GA;;;{sid})S:(ML;;NW;;;ME)"
    ))
}
fn protect_path(path: &Path, sid: &str, set_owner: bool) -> Result<(), String> {
    let sec = user_file_security(sid)?;
    let raw = unsafe {
        CreateFileW(
            wide(path).as_ptr(),
            READ_CONTROL | FILE_READ_ATTRIBUTES | WRITE_DAC | WRITE_OWNER,
            FILE_SHARE_READ,
            std::ptr::null(),
            OPEN_EXISTING,
            FILE_FLAG_BACKUP_SEMANTICS | FILE_FLAG_OPEN_REPARSE_POINT,
            std::ptr::null_mut(),
        )
    };
    if raw == INVALID_HANDLE_VALUE {
        return Err(error("打开待保护对象失败"));
    }
    let handle = Handle(raw);
    unsafe {
        let mut info: BY_HANDLE_FILE_INFORMATION = std::mem::zeroed();
        if GetFileType(handle.0) != FILE_TYPE_DISK
            || GetFileInformationByHandle(handle.0, &mut info) == 0
        {
            return Err(error("读取待保护对象类型失败"));
        }
        if info.dwFileAttributes & FILE_ATTRIBUTE_REPARSE_POINT != 0
            || (info.dwFileAttributes & FILE_ATTRIBUTE_DIRECTORY == 0 && info.nNumberOfLinks != 1)
        {
            return Err("待保护对象类型不符，或为重解析点/多硬链接文件".into());
        }
        let mut owner = std::ptr::null_mut();
        let mut dacl = std::ptr::null_mut();
        let mut sacl = std::ptr::null_mut();
        let mut present = 0;
        let mut defaulted = 0;
        if GetSecurityDescriptorOwner(sec.descriptor, &mut owner, &mut defaulted) == 0
            || GetSecurityDescriptorDacl(sec.descriptor, &mut present, &mut dacl, &mut defaulted)
                == 0
            || GetSecurityDescriptorSacl(sec.descriptor, &mut present, &mut sacl, &mut defaulted)
                == 0
        {
            return Err(error("读取私有安全描述符失败"));
        }
        let mut actual_owner = std::ptr::null_mut();
        let mut descriptor = std::ptr::null_mut();
        let status = GetSecurityInfo(
            handle.0,
            SE_FILE_OBJECT,
            OWNER_SECURITY_INFORMATION,
            &mut actual_owner,
            std::ptr::null_mut(),
            std::ptr::null_mut(),
            std::ptr::null_mut(),
            &mut descriptor,
        );
        if status != ERROR_SUCCESS {
            return Err(format!(
                "读取待保护对象所有者失败：{}",
                std::io::Error::from_raw_os_error(status as i32)
            ));
        }
        let same_owner = !actual_owner.is_null() && EqualSid(actual_owner, owner) != 0;
        LocalFree(descriptor);
        if !same_owner {
            return Err("待保护对象不属于指定用户，拒绝修改 ACL".into());
        }
        // 校验与设权使用同一无写入/删除共享句柄，不重新解析用户可替换路径。
        let mut flags = DACL_SECURITY_INFORMATION
            | PROTECTED_DACL_SECURITY_INFORMATION
            | LABEL_SECURITY_INFORMATION;
        if set_owner {
            flags |= OWNER_SECURITY_INFORMATION;
        }
        let status = SetSecurityInfo(
            handle.0,
            SE_FILE_OBJECT,
            flags,
            if set_owner {
                owner
            } else {
                std::ptr::null_mut()
            },
            std::ptr::null_mut(),
            dacl,
            sacl,
        );
        if status != ERROR_SUCCESS {
            return Err(format!(
                "设置私有文件 ACL 失败：{}",
                std::io::Error::from_raw_os_error(status as i32)
            ));
        }
    }
    Ok(())
}
pub fn protect_user_owner(path: &Path, sid: &str) -> Result<(), String> {
    protect_path(path, sid, true)
}
pub fn protect(path: &Path) -> Result<(), String> {
    protect_path(path, &user_sid()?, false)
}

pub struct MutexGuard(Handle);
impl Drop for MutexGuard {
    fn drop(&mut self) {
        unsafe {
            ReleaseMutex(self.0 .0);
        }
    }
}
pub fn mutex_held_by_other(handle: &Handle) -> Result<bool, String> {
    match unsafe { WaitForSingleObject(handle.0, 0) } {
        WAIT_TIMEOUT => Ok(true),
        WAIT_OBJECT_0 | WAIT_ABANDONED => {
            unsafe {
                ReleaseMutex(handle.0);
            }
            Ok(false)
        }
        _ => Err(error("无法确认守护恢复锁状态")),
    }
}
pub fn mutex(name: &str) -> Result<Handle, String> {
    let mut sec = Security::ipc(&user_sid()?)?;
    let raw = unsafe { CreateMutexW(&mut sec.attributes, 0, wide(name).as_ptr()) };
    if raw.is_null() {
        return Err(error("创建恢复锁失败"));
    }
    Ok(Handle(raw))
}
pub fn lock(name: &str, timeout: u32) -> Result<MutexGuard, String> {
    let h = mutex(name)?;
    match unsafe { WaitForSingleObject(h.0, timeout) } {
        WAIT_OBJECT_0 | WAIT_ABANDONED => Ok(MutexGuard(h)),
        WAIT_TIMEOUT => Err("已有实例或守护正在清理，请等待其完成后再启动".into()),
        _ => Err(error("取得恢复锁失败")),
    }
}
pub fn domain(purpose: &str) -> Result<String, String> {
    #[cfg(test)]
    if let Ok(scope) = std::env::var("SMARTCLASH_FIXTURE_SCOPE") {
        return Ok(format!("Local\\SmartClash.test.{scope}.{purpose}"));
    }
    Ok(format!("Global\\SmartClash.{}.{}", user_sid()?, purpose))
}
pub fn event_name(nonce: &str, suffix: &str) -> String {
    format!("Global\\SmartClash.{nonce}.{suffix}")
}
pub fn event(nonce: &str, suffix: &str, create: bool) -> Result<Handle, String> {
    let name = wide(event_name(nonce, suffix));
    let raw = if create {
        let mut sec = Security::ipc(&user_sid()?)?;
        let h = unsafe { CreateEventW(&mut sec.attributes, 1, 0, name.as_ptr()) };
        if !h.is_null() && unsafe { GetLastError() } == ERROR_ALREADY_EXISTS {
            unsafe {
                CloseHandle(h);
            }
            return Err("会话事件已存在，拒绝复用".into());
        }
        h
    } else {
        unsafe { OpenEventW(EVENT_MODIFY_STATE | SYNCHRONIZE, 0, name.as_ptr()) }
    };
    if raw.is_null() {
        Err(error("打开会话事件失败"))
    } else {
        Ok(Handle(raw))
    }
}
pub fn signal(event: &Handle) -> Result<(), String> {
    if unsafe { SetEvent(event.0) } == 0 {
        Err(error("发送停止确认失败"))
    } else {
        Ok(())
    }
}
pub fn nonce() -> Result<String, String> {
    use windows_sys::Win32::Security::Cryptography::*;
    let mut bytes = [0u8; 32];
    if unsafe {
        BCryptGenRandom(
            std::ptr::null_mut(),
            bytes.as_mut_ptr(),
            bytes.len() as u32,
            BCRYPT_USE_SYSTEM_PREFERRED_RNG,
        )
    } < 0
    {
        return Err("系统随机数生成失败".into());
    }
    Ok(bytes.iter().map(|b| format!("{b:02x}")).collect())
}
pub fn valid_nonce(value: &str) -> bool {
    value.len() == 64
        && value
            .bytes()
            .all(|c| c.is_ascii_hexdigit() && !c.is_ascii_uppercase())
}

pub fn atomic_write(path: &Path, data: &[u8]) -> Result<(), String> {
    use std::io::Write;
    use std::os::windows::io::FromRawHandle;
    let path = std::path::absolute(path).map_err(|e| e.to_string())?;
    path.parent().ok_or("恢复记录缺少父目录")?;
    path.file_name().ok_or("恢复记录缺少文件名")?;
    let temp = path.with_extension(format!("{}.tmp", &nonce()?[..16]));
    // 提权令牌的默认 Owner 可能是 BA；创建时显式指定用户 Owner 和中完整性标签。
    let sec = user_file_security(&user_sid()?)?;
    let raw = unsafe {
        CreateFileW(
            wide(&temp).as_ptr(),
            GENERIC_WRITE | DELETE,
            FILE_SHARE_READ,
            &sec.attributes,
            CREATE_NEW,
            FILE_ATTRIBUTE_NORMAL | FILE_FLAG_OPEN_REPARSE_POINT,
            std::ptr::null_mut(),
        )
    };
    if raw == INVALID_HANDLE_VALUE {
        return Err(error("原子创建私有临时文件失败"));
    }
    let mut file = unsafe { std::fs::File::from_raw_handle(raw.cast()) };
    let result = (|| {
        file.write_all(data)
            .and_then(|_| file.sync_all())
            .map_err(|e| e.to_string())?;
        let target = wide(&path);
        let size = std::mem::size_of::<FILE_RENAME_INFO>() + target.len() * 2;
        let mut buffer = vec![0usize; size.div_ceil(std::mem::size_of::<usize>())];
        let rename = buffer.as_mut_ptr().cast::<FILE_RENAME_INFO>();
        // 保持无删除共享的源句柄直至重命名完成，不留下关闭后按路径重开的窗口。
        unsafe {
            (*rename).Anonymous.ReplaceIfExists = true;
            (*rename).FileNameLength = ((target.len() - 1) * 2) as u32;
            std::ptr::copy_nonoverlapping(
                target.as_ptr(),
                std::ptr::addr_of_mut!((*rename).FileName).cast::<u16>(),
                target.len(),
            );
            if SetFileInformationByHandle(
                file.as_raw_handle().cast(),
                FileRenameInfo,
                rename.cast(),
                size as u32,
            ) == 0
            {
                return Err(error("原子替换恢复记录失败"));
            }
        }
        Ok(())
    })();
    if result.is_err() {
        let disposition = FILE_DISPOSITION_INFO { DeleteFile: true };
        unsafe {
            SetFileInformationByHandle(
                file.as_raw_handle().cast(),
                FileDispositionInfo,
                (&disposition as *const FILE_DISPOSITION_INFO).cast(),
                std::mem::size_of_val(&disposition) as u32,
            );
        }
    }
    result
}

pub struct Job(Handle);
unsafe impl Send for Job {}
unsafe impl Sync for Job {}
#[link(name = "ntdll")]
extern "system" {
    fn NtResumeProcess(process: HANDLE) -> i32;
}
impl Job {
    pub fn new() -> Result<Self, String> {
        Self::create(None)
    }
    pub fn named(name: &str) -> Result<Self, String> {
        Self::create(Some(name))
    }
    fn create(name: Option<&str>) -> Result<Self, String> {
        let mut sec = Security::ipc(&user_sid()?)?;
        let name = name.map(wide);
        let raw = unsafe {
            CreateJobObjectW(
                &mut sec.attributes,
                name.as_ref().map_or(std::ptr::null(), |n| n.as_ptr()),
            )
        };
        if name.is_some() && !raw.is_null() && unsafe { GetLastError() } == ERROR_ALREADY_EXISTS {
            unsafe {
                CloseHandle(raw);
            }
            return Err("内核 Job 名称已存在，拒绝复用".into());
        }
        if raw.is_null() {
            return Err(error("创建内核 Job 失败"));
        }
        let h = Handle(raw);
        let mut limits: JOBOBJECT_EXTENDED_LIMIT_INFORMATION = unsafe { std::mem::zeroed() };
        limits.BasicLimitInformation.LimitFlags = JOB_OBJECT_LIMIT_KILL_ON_JOB_CLOSE;
        if unsafe {
            SetInformationJobObject(
                h.0,
                JobObjectExtendedLimitInformation,
                (&limits as *const JOBOBJECT_EXTENDED_LIMIT_INFORMATION).cast(),
                std::mem::size_of_val(&limits) as u32,
            )
        } == 0
        {
            return Err(error("设置内核 Job 失败"));
        }
        Ok(Self(h))
    }
    pub fn assign_current(&self) -> Result<(), String> {
        if unsafe { AssignProcessToJobObject(self.0 .0, GetCurrentProcess()) } == 0 {
            return Err(error("GUI 无法加入生命周期 Job"));
        }
        Ok(())
    }
    pub fn spawn_atomic(
        &self,
        exe: &Path,
        args: &[String],
        dir: &Path,
        file: &std::fs::File,
    ) -> Result<Handle, String> {
        use std::os::windows::io::AsRawHandle;
        let log_handle = file.as_raw_handle() as HANDLE;
        unsafe {
            if SetHandleInformation(log_handle, HANDLE_FLAG_INHERIT, HANDLE_FLAG_INHERIT) == 0 {
                return Err(error("设置日志句柄失败"));
            }
            let mut size = 0;
            InitializeProcThreadAttributeList(std::ptr::null_mut(), 2, 0, &mut size);
            let mut buffer = vec![0usize; (size + 7) / 8];
            let list = buffer.as_mut_ptr().cast();
            if InitializeProcThreadAttributeList(list, 2, 0, &mut size) == 0 {
                return Err(error("初始化原子 Job 属性失败"));
            }
            struct Attributes(LPPROC_THREAD_ATTRIBUTE_LIST);
            impl Drop for Attributes {
                fn drop(&mut self) {
                    unsafe {
                        DeleteProcThreadAttributeList(self.0);
                    }
                }
            }
            let _attributes = Attributes(list);
            let mut job = self.0 .0;
            let mut log_handle_copy = log_handle;
            if UpdateProcThreadAttribute(
                list,
                0,
                0x0002_000d,
                (&mut job as *mut HANDLE).cast(),
                std::mem::size_of::<HANDLE>(),
                std::ptr::null_mut(),
                std::ptr::null_mut(),
            ) == 0
            {
                return Err(error("设置原子 Job 列表失败"));
            }
            if UpdateProcThreadAttribute(
                list,
                0,
                0x0002_0002,
                (&mut log_handle_copy as *mut HANDLE).cast(),
                std::mem::size_of::<HANDLE>(),
                std::ptr::null_mut(),
                std::ptr::null_mut(),
            ) == 0
            {
                return Err(error("设置受限继承句柄失败"));
            }
            let mut startup: STARTUPINFOEXW = std::mem::zeroed();
            startup.StartupInfo.cb = std::mem::size_of_val(&startup) as u32;
            startup.StartupInfo.dwFlags = STARTF_USESTDHANDLES;
            startup.StartupInfo.hStdOutput = log_handle;
            startup.StartupInfo.hStdError = log_handle;
            startup.lpAttributeList = list;
            fn quote(arg: &str) -> String {
                let mut out = String::from("\"");
                let mut slashes = 0;
                for c in arg.chars() {
                    if c == '\\' {
                        slashes += 1;
                        continue;
                    }
                    if c == '"' {
                        out.push_str(&"\\".repeat(slashes * 2 + 1));
                    } else {
                        out.push_str(&"\\".repeat(slashes));
                    }
                    slashes = 0;
                    out.push(c);
                }
                out.push_str(&"\\".repeat(slashes * 2));
                out.push('"');
                out
            }
            let mut command_line = wide(
                std::iter::once(exe.to_string_lossy().into_owned())
                    .chain(args.iter().cloned())
                    .map(|a| quote(&a))
                    .collect::<Vec<_>>()
                    .join(" "),
            );
            let mut info: PROCESS_INFORMATION = std::mem::zeroed();
            if CreateProcessW(
                wide(exe).as_ptr(),
                command_line.as_mut_ptr(),
                std::ptr::null(),
                std::ptr::null(),
                1,
                EXTENDED_STARTUPINFO_PRESENT | CREATE_NO_WINDOW,
                std::ptr::null(),
                wide(dir).as_ptr(),
                &startup.StartupInfo,
                &mut info,
            ) == 0
            {
                return Err(error("原子 Job 内核启动失败"));
            }
            CloseHandle(info.hThread);
            Ok(Handle(info.hProcess))
        }
    }
    pub fn spawn(&self, command: &mut Command) -> Result<Child, String> {
        // GUI 已先加入 KILL_ON_JOB_CLOSE Job；CreateProcess 返回前即继承外层约束。
        // 因而在这里加入探测专用内层 Job 之前崩溃，也不会留下无所属子进程。
        command.creation_flags(CREATE_SUSPENDED | CREATE_NO_WINDOW);
        let mut child = command.spawn().map_err(|e| e.to_string())?;
        let handle = child.as_raw_handle() as HANDLE;
        if unsafe { AssignProcessToJobObject(self.0 .0, handle) } == 0 {
            let err = error("内核无法加入 Job，拒绝无约束运行");
            let _ = child.kill();
            let _ = child.wait();
            return Err(err);
        }
        if unsafe { NtResumeProcess(handle) } < 0 {
            let _ = child.kill();
            let _ = child.wait();
            return Err("恢复受控子进程失败".into());
        }
        Ok(child)
    }
    pub fn terminate(&self) -> Result<(), String> {
        if unsafe { TerminateJobObject(self.0 .0, 1) } == 0 {
            return Err(error("结束内核 Job 失败"));
        }
        self.wait_empty()
    }
    pub fn wait_named_empty(name: &str) -> Result<(), String> {
        let raw = unsafe { OpenJobObjectW(0x0004, 0, wide(name).as_ptr()) };
        if raw.is_null() {
            if unsafe { GetLastError() } == ERROR_FILE_NOT_FOUND {
                return Ok(());
            }
            return Err(error("无法确认遗留 Job 状态"));
        }
        Self(Handle(raw)).wait_empty()
    }
    pub fn wait_empty(&self) -> Result<(), String> {
        let end = std::time::Instant::now() + std::time::Duration::from_secs(15);
        loop {
            let mut info: JOBOBJECT_BASIC_ACCOUNTING_INFORMATION = unsafe { std::mem::zeroed() };
            if unsafe {
                QueryInformationJobObject(
                    self.0 .0,
                    JobObjectBasicAccountingInformation,
                    (&mut info as *mut JOBOBJECT_BASIC_ACCOUNTING_INFORMATION).cast(),
                    std::mem::size_of_val(&info) as u32,
                    std::ptr::null_mut(),
                )
            } == 0
            {
                return Err(error("查询 Job 回收结果失败"));
            }
            if info.ActiveProcesses == 0 {
                return Ok(());
            }
            if std::time::Instant::now() >= end {
                return Err("内核 Job 尚未清理完成".into());
            }
            std::thread::sleep(std::time::Duration::from_millis(20));
        }
    }
}

pub fn elevate(exe: &Path, args: &str) -> Result<Handle, String> {
    use windows_sys::Win32::UI::{Shell::*, WindowsAndMessaging::SW_HIDE};
    let verb = wide("runas");
    let file = wide(exe);
    let args = wide(args);
    let mut info: SHELLEXECUTEINFOW = unsafe { std::mem::zeroed() };
    info.cbSize = std::mem::size_of_val(&info) as u32;
    info.fMask = SEE_MASK_NOCLOSEPROCESS | SEE_MASK_NOASYNC;
    info.lpVerb = verb.as_ptr();
    info.lpFile = file.as_ptr();
    info.lpParameters = args.as_ptr();
    info.nShow = SW_HIDE;
    if unsafe { ShellExecuteExW(&mut info) } == 0 {
        return Err(error("管理员授权未完成（可能取消 UAC）"));
    }
    if info.hProcess.is_null() {
        return Err("提权 broker 未返回进程句柄".into());
    }
    Ok(Handle(info.hProcess))
}
