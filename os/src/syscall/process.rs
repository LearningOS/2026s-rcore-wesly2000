//! Process management syscalls

use alloc::sync::Arc;

use crate::{
    loader::get_app_data_by_name,
    mm::{translated_refmut, translated_str},
    task::{
        add_task, current_task, current_user_token, exit_current_and_run_next,
        suspend_current_and_run_next,
    },
, current_user_token};
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

    // Use translated_byte_buffer to handle cross-page writes safely
    let token = current_user_token();
    let mut buffers = translated_byte_buffer(token, ts as *const u8, size_of::<TimeVal>());

    // Write sec and usec through the translated buffers
    let real_ts = buffers[0].as_mut_ptr() as *mut TimeVal;

    unsafe { *(real_ts) = TimeVal { sec, usec }};
    // let mut offset = 0;
    // let timeval_bytes = [
    //     &sec.to_ne_bytes()[..],
    //     &usec.to_ne_bytes()[..],
    // ].concat();

    // for buffer in buffers {
    //     let copy_len = buffer.len().min(timeval_bytes.len() - offset);
    //     buffer[..copy_len].copy_from_slice(&timeval_bytes[offset..offset + copy_len]);
    //     offset += copy_len;
    // }

    0
}

/// TODO: Finish sys_trace to pass testcases
/// HINT: You might reimplement it with virtual memory management.
pub fn sys_trace(_trace_request: usize, _id: usize, _data: usize) -> isize {
    trace!("kernel: sys_trace");
    -1
}


const PROT_R: usize = 1;
const PROT_W: usize = 1 << 1;
const PROT_X: usize = 1 << 2;
// YOUR JOB: Implement mmap.
pub fn sys_mmap(start: usize, len: usize, prot: usize) -> isize {
    let start_va = VirtAddr::from(start);
    let end_va = VirtAddr::from(start + len);

    let start_vpn: usize = usize::from(start_va.floor());
    let end_vpn: usize = usize::from(end_va.ceil());

    if !start_va.aligned() { 
        trace!("Not aligned to page size");
        return -1; 
    }  
    if prot & !0x7 != 0 { 
        trace!("Bits need to be 0 except for the lowest 3 bits");
        return -1; 
    }         
    if prot & 0x7 == 0 { 
        trace!("Meaningless allocation");
        return -1; 
    }

    let token = current_user_token();
    let mut page_table = PageTable::from_token(token);

    for vpn in start_vpn..end_vpn {
        if let Some(_) = page_table.translate(VirtPageNum::from(vpn)) {
            trace!("Virtual address has been processed");
            return -1;
        }
    }

    let mut flags: PTEFlags = PTEFlags::U;
    if prot & PROT_R > 0 {
        flags |= PTEFlags::R;
    }
    if prot & PROT_W > 0 {
        flags |= PTEFlags::W;
    }
    if prot & PROT_X > 0 {
        flags |= PTEFlags::X;
    }

    /* 
     * Allocate enough PhysPage and push the ppns into a vector, once the allocator returns None,
     * it means that no enough space and we should return -1. Note that the allocator does not 
     * return ppn directly, but through FrameTracker.
     * 
     * If all PhysPages are successfully allocated, we need to map them one by one.
     * 
     * NOTE: We don't consider the situation that if map fails we need to recycle the allocated pages.
     */

    for vpn in start_vpn..end_vpn {
        match frame_alloc() {
            Some(tracker) => { 
                page_table.map(VirtPageNum::from(vpn), tracker.ppn, flags);
             },
            None => { 
                trace!("No enough space");
                return -1; 
            }
        } 
    }

    0
}

/// YOUR JOB: Implement munmap.
pub fn sys_munmap(_start: usize, _len: usize) -> isize {
    trace!(
        "kernel:pid[{}] sys_munmap NOT IMPLEMENTED",
        current_task().unwrap().pid.0
    );
    -1
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
