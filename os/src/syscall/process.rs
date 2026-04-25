//! Process management syscalls

use alloc::sync::Arc;

use crate::{
    loader::get_app_data_by_name,
    mm::{translated_refmut, translated_str},
    task::{
        add_task, current_task, current_user_token, current_user_token, exit_current_and_run_next, get_current_syscall_count,
        mmap_current_task, munmap_current_task, suspend_current_and_run_next,
    },
};
use crate::mm::*;
use crate::timer::get_time_us;
use core::mem::size_of;

#[repr(C)]
#[derive(Debug)]
pub struct TimeVal {
    pub sec: usize,
    pub usec: usize,
}

/// task exits and submit an exit code
pub fn sys_exit(exit_code: i32) -> ! {
    trace!("kernel:pid[{}] sys_exit", current_task().unwrap().pid.0);
    exit_current_and_run_next(exit_code);
    panic!("Unreachable in sys_exit!");
}

/// current task gives up resources for other tasks
pub fn sys_yield() -> isize {
    trace!("kernel:pid[{}] sys_yield", current_task().unwrap().pid.0);
    suspend_current_and_run_next();
    0
}

/// Write to address with len bytes from `src`. It handles user page table fetching,
/// va to pa translation where segmentation might exist.
fn write_mem(ptr: *const u8, src: &[u8]) {
    let token = current_user_token();
    let buffers = translated_byte_buffer(token, ptr, src.len());

    let mut offset = 0;

    for buffer in buffers {
        let copy_len = buffer.len().min(src.len() - offset);
        buffer[..copy_len].copy_from_slice(&src[offset..offset + copy_len]);
        offset += copy_len;
    }
}


/// Read from address with len bytes to `dst`. It handles user page table fetching,
/// va to pa translation where segmentation might exist.
fn read_mem(ptr: *const u8, dst: &mut [u8]) {
    let token = current_user_token();
    let buffers = translated_byte_buffer(token, ptr, dst.len());
    let mut offset = 0;

    for buffer in buffers {
        let copy_len = buffer.len().min(dst.len() - offset);
        dst[offset..offset + copy_len].copy_from_slice(&buffer[..copy_len]);
        offset += copy_len;
    }
}

/// Max avaiable addr that the user could touch
const MAX_ADDR: usize = 0x0000_003f_ffff_ffff;

/// Check if an address to Read/Write is valid (through page table).
/// 
/// op indicates the operation: Read (1) / Write (1 << 1) / Executtion (1 << 2), 
/// consistent with the format of prot.
fn addr_check(start: usize, len: usize, op: usize) -> bool {
    if !MAX_ADDR & start > 0 {
        error!("Address not allowed in user space");
        return false;
    } 
    if op & !(PROT_R|PROT_W|PROT_X) != 0 {
        error!("Invalid operation");
        return false;
    }

    let start_va = VirtAddr::from(start);
    let end_va = VirtAddr::from(start + len);

    let start_vpn: usize = usize::from(start_va.floor());
    let end_vpn: usize = usize::from(end_va.ceil());

    let token = current_user_token();
    let page_table = PageTable::from_token(token);

    for vpn in start_vpn..end_vpn {
        match page_table.translate(VirtPageNum::from(vpn)) {
            None => {
                error!("Virtual page is never in use");
                return false;
            }
            Some(pte) => {
                if !pte.is_valid() {
                    error!("Virtual page is not valid");
                    return false;
                }
                if !pte.user_available() {
                    error!("Page could not be accessed by user");
                    return false;
                }
                if ((op & PROT_R > 0) && !pte.readable())
                || ((op & PROT_W > 0) && !pte.writable())
                || ((op & PROT_X > 0) && !pte.executable()) {
                    error!("Operation is now allowed");
                    return false;
                }
            }
        }
    }

    true
}


pub fn sys_getpid() -> isize {
    trace!("kernel: sys_getpid pid:{}", current_task().unwrap().pid.0);
    current_task().unwrap().pid.0 as isize
}

pub fn sys_fork() -> isize {
    trace!("kernel:pid[{}] sys_fork", current_task().unwrap().pid.0);
    let current_task = current_task().unwrap();
    let new_task = current_task.fork();
    let new_pid = new_task.pid.0;
    // modify trap context of new_task, because it returns immediately after switching
    let trap_cx = new_task.inner_exclusive_access().get_trap_cx();
    // we do not have to move to next instruction since we have done it before
    // for child process, fork returns 0
    trap_cx.x[10] = 0;
    // add new task to scheduler
    add_task(new_task);
    new_pid as isize
}

pub fn sys_exec(path: *const u8) -> isize {
    trace!("kernel:pid[{}] sys_exec", current_task().unwrap().pid.0);
    let token = current_user_token();
    let path = translated_str(token, path);
    if let Some(data) = get_app_data_by_name(path.as_str()) {
        let task = current_task().unwrap();
        task.exec(data);
        0
    } else {
        -1
    }
}

/// If there is not a child process whose pid is same as given, return -1.
/// Else if there is a child process but it is still running, return -2.
pub fn sys_waitpid(pid: isize, exit_code_ptr: *mut i32) -> isize {
    trace!("kernel::pid[{}] sys_waitpid [{}]", current_task().unwrap().pid.0, pid);
    let task = current_task().unwrap();
    // find a child process

    // ---- access current PCB exclusively
    let mut inner = task.inner_exclusive_access();
    if !inner
        .children
        .iter()
        .any(|p| pid == -1 || pid as usize == p.getpid())
    {
        return -1;
        // ---- release current PCB
    }
    let pair = inner.children.iter().enumerate().find(|(_, p)| {
        // ++++ temporarily access child PCB exclusively
        p.inner_exclusive_access().is_zombie() && (pid == -1 || pid as usize == p.getpid())
        // ++++ release child PCB
    });
    if let Some((idx, _)) = pair {
        let child = inner.children.remove(idx);
        // confirm that child will be deallocated after being removed from children list
        assert_eq!(Arc::strong_count(&child), 1);
        let found_pid = child.getpid();
        // ++++ temporarily access child PCB exclusively
        let exit_code = child.inner_exclusive_access().exit_code;
        // ++++ release child PCB
        *translated_refmut(inner.memory_set.token(), exit_code_ptr) = exit_code;
        found_pid as isize
    } else {
        -2
    }
    // ---- release current PCB automatically
}

/// YOUR JOB: get time with second and microsecond
/// HINT: You might reimplement it with virtual memory management.
/// HINT: What if [`TimeVal`] is splitted by two pages ?
pub fn sys_get_time(ts: *mut TimeVal, _tz: usize) -> isize {
    trace!("kernel: sys_get_time");

    let us = get_time_us();
    let sec = us / 1_000_000;
    let usec = us % 1_000_000;
    
    let timeval_bytes = [
        &sec.to_ne_bytes()[..],
        &usec.to_ne_bytes()[..],
    ].concat();

    write_mem(ts as *const u8, &timeval_bytes);

    0
}

/// TODO: Finish sys_trace to pass testcases
/// HINT: You might reimplement it with virtual memory management.
pub fn sys_trace(trace_request: usize, id: usize, data: usize) -> isize {
    trace!("kernel: sys_trace");
    const LEN: usize = size_of::<usize>();
    match trace_request {
        0 => { 
                if !addr_check(id, LEN, PROT_R) {
                    error!("Invalid read address");
                    return -1;
                }
                let mut buffer: [u8; LEN] = [0; LEN];
                read_mem(id as *const u8, &mut buffer);
                return usize::from_ne_bytes(buffer) as isize; 
        },
        1 => {
                if !addr_check(id, LEN, PROT_W) {
                    error!("Invalid write address");
                    return -1;
                }
                let buffer = &data.to_ne_bytes()[..];
                write_mem(id as *const u8, &buffer);
                return 0;
        },
        2 => {
            // Return the syscall count in current task
            if let Some(cnt) = get_current_syscall_count(id) {
                return cnt as isize;
            }
        },
        _ => { return -1; },
    }
    -1
}

// YOUR JOB: Implement mmap.
pub fn sys_mmap(start: usize, len: usize, prot: usize) -> isize {
    mmap_current_task(start, len, prot)
}

// YOUR JOB: Implement munmap.
pub fn sys_munmap(start: usize, len: usize) -> isize {
    munmap_current_task(start, len)
}

/// change data segment size
pub fn sys_sbrk(size: i32) -> isize {
    trace!("kernel:pid[{}] sys_sbrk", current_task().unwrap().pid.0);
    if let Some(old_brk) = current_task().unwrap().change_program_brk(size) {
        old_brk as isize
    } else {
        -1
    }
}

/// YOUR JOB: Implement spawn.
/// HINT: fork + exec =/= spawn
pub fn sys_spawn(_path: *const u8) -> isize {
    trace!(
        "kernel:pid[{}] sys_spawn NOT IMPLEMENTED",
        current_task().unwrap().pid.0
    );
    -1
}

// YOUR JOB: Set task priority.
pub fn sys_set_priority(_prio: isize) -> isize {
    trace!(
        "kernel:pid[{}] sys_set_priority NOT IMPLEMENTED",
        current_task().unwrap().pid.0
    );
    -1
}
