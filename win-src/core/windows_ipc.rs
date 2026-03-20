use anyhow::Result;
use std::io::{Read, Write};
use std::os::windows::io::{AsRawHandle, RawHandle};
use std::sync::atomic::{AtomicPtr, Ordering};
use std::time::Duration;

use windows::Win32::Foundation::*;
use windows::Win32::Storage::FileSystem::*;
use windows::Win32::System::Pipes::*;
use windows::Win32::System::IO::*;
use windows::Win32::System::Threading::{CreateEventW, WaitForSingleObject};

pub const PIPE_NAME: &str = "wftpd";

fn get_pipe_path() -> String {
    format!("\\\\.\\pipe\\{}", PIPE_NAME)
}

pub struct IpcServerInner {
    handle: AtomicPtr<std::ffi::c_void>,
}

// SAFETY: IpcServerInner wraps a Windows HANDLE which is valid to send across
// threads. All access goes through AtomicPtr with SeqCst ordering.
unsafe impl Send for IpcServerInner {}
unsafe impl Sync for IpcServerInner {}

impl IpcServerInner {
    pub fn new() -> Result<Self> {
        let pipe_path: Vec<u16> = get_pipe_path().encode_utf16().chain(std::iter::once(0)).collect();
        
        unsafe {
            let handle = CreateNamedPipeW(
                windows::core::PCWSTR(pipe_path.as_ptr()),
                PIPE_ACCESS_DUPLEX | FILE_FLAG_OVERLAPPED,
                PIPE_TYPE_BYTE | PIPE_READMODE_BYTE | PIPE_WAIT,
                PIPE_UNLIMITED_INSTANCES,
                65536,
                65536,
                0,
                None,
            );
            
            if handle.is_invalid() {
                anyhow::bail!("Failed to create named pipe: {}", std::io::Error::last_os_error());
            }
            
            log::info!("Named pipe server created: {}", get_pipe_path());
            
            Ok(IpcServerInner { 
                handle: AtomicPtr::new(handle.0) 
            })
        }
    }
    
    pub fn accept(&self) -> Result<IpcStream> {
        unsafe {
            let event = CreateEventW(None, true, false, None)?;
            let mut overlapped = OVERLAPPED { hEvent: event, ..Default::default() };
            
            let current_handle = HANDLE(self.handle.load(Ordering::SeqCst));
            
            let result = ConnectNamedPipe(current_handle, Some(&mut overlapped));
            
            match result {
                Ok(()) => {}
                Err(e) if e.code() == ERROR_IO_PENDING.to_hresult() => {
                    let mut bytes_transferred: u32 = 0;
                    let success = GetOverlappedResult(current_handle, &overlapped, &mut bytes_transferred, true);
                    if success.is_err() {
                        anyhow::bail!("Failed to wait for pipe connection");
                    }
                }
                Err(e) if e.code() == ERROR_PIPE_CONNECTED.to_hresult() => {
                }
                Err(e) => {
                    anyhow::bail!("Failed to connect named pipe: {:?}", e);
                }
            }
            
            let new_handle = CreateNamedPipeW(
                windows::core::PCWSTR(get_pipe_path().encode_utf16().chain(std::iter::once(0)).collect::<Vec<u16>>().as_ptr()),
                PIPE_ACCESS_DUPLEX | FILE_FLAG_OVERLAPPED,
                PIPE_TYPE_BYTE | PIPE_READMODE_BYTE | PIPE_WAIT,
                PIPE_UNLIMITED_INSTANCES,
                65536,
                65536,
                0,
                None,
            );
            
            self.handle.store(new_handle.0, Ordering::SeqCst);
            
            Ok(IpcStream { handle: current_handle })
        }
    }
    
    pub fn accept_timeout(&self, timeout: Duration) -> Result<Option<IpcStream>> {
        unsafe {
            let event = CreateEventW(None, true, false, None)?;
            let mut overlapped = OVERLAPPED { hEvent: event, ..Default::default() };
            
            let current_handle = HANDLE(self.handle.load(Ordering::SeqCst));
            
            let result = ConnectNamedPipe(current_handle, Some(&mut overlapped));
            
            match result {
                Ok(()) => {}
                Err(e) if e.code() == ERROR_IO_PENDING.to_hresult() => {
                    let wait_result = WaitForSingleObject(event, timeout.as_millis() as u32);
                    if wait_result == WAIT_TIMEOUT {
                        CancelIo(current_handle)?;
                        return Ok(None);
                    }
                    
                    let mut bytes_transferred: u32 = 0;
                    let success = GetOverlappedResult(current_handle, &overlapped, &mut bytes_transferred, false);
                    if success.is_err() {
                        anyhow::bail!("Failed to wait for pipe connection");
                    }
                }
                Err(e) if e.code() == ERROR_PIPE_CONNECTED.to_hresult() => {
                }
                Err(e) => {
                    anyhow::bail!("Failed to connect named pipe: {:?}", e);
                }
            }
            
            let new_handle = CreateNamedPipeW(
                windows::core::PCWSTR(get_pipe_path().encode_utf16().chain(std::iter::once(0)).collect::<Vec<u16>>().as_ptr()),
                PIPE_ACCESS_DUPLEX | FILE_FLAG_OVERLAPPED,
                PIPE_TYPE_BYTE | PIPE_READMODE_BYTE | PIPE_WAIT,
                PIPE_UNLIMITED_INSTANCES,
                65536,
                65536,
                0,
                None,
            );
            
            self.handle.store(new_handle.0, Ordering::SeqCst);
            
            Ok(Some(IpcStream { handle: current_handle }))
        }
    }
}

impl Drop for IpcServerInner {
    fn drop(&mut self) {
        unsafe {
            let handle = HANDLE(self.handle.load(Ordering::SeqCst));
            if !handle.is_invalid() {
                let _ = CloseHandle(handle);
            }
        }
    }
}

pub struct IpcStream {
    handle: HANDLE,
}

unsafe impl Send for IpcStream {}

impl IpcStream {
    pub fn connect() -> Result<Self> {
        let pipe_path: Vec<u16> = get_pipe_path().encode_utf16().chain(std::iter::once(0)).collect();
        
        unsafe {
            let mut attempts = 0;
            const MAX_ATTEMPTS: u32 = 3;
            loop {
                let result = WaitNamedPipeW(
                    windows::core::PCWSTR(pipe_path.as_ptr()),
                    500,
                );
                
                if !result.as_bool() {
                    attempts += 1;
                    if attempts >= MAX_ATTEMPTS {
                        anyhow::bail!("命名管道服务不可用，服务可能未启动，请等待几秒后重试");
                    }
                    std::thread::sleep(Duration::from_millis(200));
                    continue;
                }
                break;
            }
            
            let handle = CreateFileW(
                windows::core::PCWSTR(pipe_path.as_ptr()),
                (GENERIC_READ | GENERIC_WRITE).0,
                FILE_SHARE_READ | FILE_SHARE_WRITE,
                None,
                OPEN_EXISTING,
                FILE_ATTRIBUTE_NORMAL,
                None,
            )?;
            
            if handle.is_invalid() {
                anyhow::bail!("Failed to connect to pipe: {}", std::io::Error::last_os_error());
            }
            
            Ok(IpcStream { handle })
        }
    }
}

impl Read for &IpcStream {
    fn read(&mut self, buf: &mut [u8]) -> std::io::Result<usize> {
        unsafe {
            let mut bytes_read: u32 = 0;
            let result = ReadFile(
                self.handle,
                Some(buf),
                Some(&mut bytes_read),
                None,
            );
            
            match result {
                Ok(()) => Ok(bytes_read as usize),
                Err(e) => Err(std::io::Error::other(e)),
            }
        }
    }
}

impl Write for &IpcStream {
    fn write(&mut self, buf: &[u8]) -> std::io::Result<usize> {
        unsafe {
            let mut bytes_written: u32 = 0;
            let result = WriteFile(
                self.handle,
                Some(buf),
                Some(&mut bytes_written),
                None,
            );
            
            match result {
                Ok(()) => Ok(bytes_written as usize),
                Err(e) => Err(std::io::Error::other(e)),
            }
        }
    }
    
    fn flush(&mut self) -> std::io::Result<()> {
        unsafe {
            let result = FlushFileBuffers(self.handle);
            match result {
                Ok(()) => Ok(()),
                Err(e) => Err(std::io::Error::other(e)),
            }
        }
    }
}

impl AsRawHandle for IpcStream {
    fn as_raw_handle(&self) -> RawHandle {
        self.handle.0 as RawHandle
    }
}

impl Drop for IpcStream {
    fn drop(&mut self) {
        unsafe {
            if !self.handle.is_invalid() {
                let _ = CloseHandle(self.handle);
            }
        }
    }
}
