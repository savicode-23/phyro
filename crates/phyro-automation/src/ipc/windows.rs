use std::{
    io,
    mem::size_of,
    os::windows::io::{AsRawHandle, FromRawHandle, OwnedHandle},
    ptr,
};
use tokio::net::windows::named_pipe::{
    ClientOptions, NamedPipeClient, NamedPipeServer, ServerOptions,
};
use windows_sys::Win32::{
    Foundation::LocalFree,
    Security::{
        Authorization::{
            ConvertSidToStringSidW, ConvertStringSecurityDescriptorToSecurityDescriptorW,
        },
        GetTokenInformation, TokenUser, SECURITY_ATTRIBUTES, TOKEN_QUERY, TOKEN_USER,
    },
    System::Threading::{GetCurrentProcess, OpenProcessToken},
};

pub type Stream = NamedPipeServer;
pub type Client = NamedPipeClient;
pub struct Listener(Stream);

fn path(locator: &str) -> String {
    format!(r"\\.\pipe\phyro-mcp-{locator}")
}

fn user_sid() -> io::Result<String> {
    // SAFETY: the process pseudo-handle is valid. The token handle, aligned buffer
    // and converted SID remain alive until their last use.
    unsafe {
        let mut raw = ptr::null_mut();
        if OpenProcessToken(GetCurrentProcess(), TOKEN_QUERY, &mut raw) == 0 {
            return Err(io::Error::last_os_error());
        }
        let token = OwnedHandle::from_raw_handle(raw);
        let mut size = 0;
        GetTokenInformation(
            token.as_raw_handle(),
            TokenUser,
            ptr::null_mut(),
            0,
            &mut size,
        );
        if size == 0 {
            return Err(io::Error::last_os_error());
        }
        let mut buffer = vec![0usize; (size as usize).div_ceil(size_of::<usize>())];
        if GetTokenInformation(
            token.as_raw_handle(),
            TokenUser,
            buffer.as_mut_ptr().cast(),
            size,
            &mut size,
        ) == 0
        {
            return Err(io::Error::last_os_error());
        }
        let sid = (*buffer.as_ptr().cast::<TOKEN_USER>()).User.Sid;
        let mut string = ptr::null_mut();
        if ConvertSidToStringSidW(sid, &mut string) == 0 {
            return Err(io::Error::last_os_error());
        }
        let mut len = 0;
        while *string.add(len) != 0 {
            len += 1;
        }
        let result = String::from_utf16_lossy(std::slice::from_raw_parts(string, len));
        LocalFree(string.cast());
        Ok(result)
    }
}

impl Listener {
    pub(super) fn bind(locator: &str) -> io::Result<Self> {
        let sddl: Vec<_> = format!("D:P(A;;GA;;;{})\0", user_sid()?)
            .encode_utf16()
            .collect();
        // SAFETY: SDDL is NUL-terminated. CreateNamedPipe copies the descriptor
        // before returning, at which point its LocalAlloc storage can be freed.
        unsafe {
            let mut descriptor = ptr::null_mut();
            if ConvertStringSecurityDescriptorToSecurityDescriptorW(
                sddl.as_ptr(),
                1,
                &mut descriptor,
                ptr::null_mut(),
            ) == 0
            {
                return Err(io::Error::last_os_error());
            }
            let mut attrs = SECURITY_ATTRIBUTES {
                nLength: size_of::<SECURITY_ATTRIBUTES>() as u32,
                lpSecurityDescriptor: descriptor,
                bInheritHandle: 0,
            };
            let pipe = ServerOptions::new()
                .first_pipe_instance(true)
                .max_instances(1)
                .reject_remote_clients(true)
                .create_with_security_attributes_raw(
                    path(locator),
                    (&mut attrs as *mut SECURITY_ATTRIBUTES).cast(),
                );
            LocalFree(descriptor);
            pipe.map(Self)
        }
    }
    pub(super) async fn accept(self) -> io::Result<Stream> {
        self.0.connect().await?;
        Ok(self.0)
    }
}

pub async fn connect(locator: &str) -> io::Result<Client> {
    ClientOptions::new().open(path(locator))
}
