//! Process management syscalls

use crate::task::{change_program_brk, exit_current_and_run_next, suspend_current_and_run_next, current_user_token};
use crate::mm::*;

#[repr(C)]
#[derive(Debug)]
pub struct TimeVal {
    pub sec: usize,
    pub usec: usize,
}

/// task exits and submit an exit code
pub fn sys_exit(_exit_code: i32) -> ! {
    trace!("kernel: sys_exit");
    exit_current_and_run_next();
    panic!("Unreachable in sys_exit!");
}

/// current task gives up resources for other tasks
pub fn sys_yield() -> isize {
    trace!("kernel: sys_yield");
    suspend_current_and_run_next();
    0
}

/// YOUR JOB: get time with second and microsecond
/// HINT: You might reimplement it with virtual memory management.
/// HINT: What if [`TimeVal`] is splitted by two pages ?
pub fn sys_get_time(_ts: *mut TimeVal, _tz: usize) -> isize {
    trace!("kernel: sys_get_time");
    -1
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

    if !start_va.aligned() { return -1; }       // Not aligned to page size
    if prot & !0x7 != 0 { return -1; }          // Bits need to be 0 except for the lowest 3 bits
    if prot & 0x7 == 0 { return -1; }           // Meaningless allocation

    let token = current_user_token();
    let mut page_table = PageTable::from_token(token);

    for vpn in start_vpn..end_vpn {
        if let Some(_) = page_table.translate(VirtPageNum::from(vpn)) {
            return -1;                          // Virtual address has been processed
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
            None => { return -1; }              // No enough space
        } 
    }

    0
}

// YOUR JOB: Implement munmap.
pub fn sys_munmap(_start: usize, _len: usize) -> isize {
    trace!("kernel: sys_munmap NOT IMPLEMENTED YET!");
    -1
}
/// change data segment size
pub fn sys_sbrk(size: i32) -> isize {
    trace!("kernel: sys_sbrk");
    if let Some(old_brk) = change_program_brk(size) {
        old_brk as isize
    } else {
        -1
    }
}
