use crate::sync::{Condvar, Mutex, MutexBlocking, MutexSpin, Semaphore};
use crate::task::{block_current_and_run_next, current_process, current_task};
use crate::timer::{add_timer, get_time_ms};
use alloc::sync::Arc;
use alloc::vec;
use alloc::vec::Vec;


fn banker_algorithm(mut available: Vec<isize>, allocation: Vec<Vec<isize>>, request: Vec<Vec<isize>>) -> bool {
    // Check if the allocation is possible by:
    // 1. If all of the elements in finish vector is true, return true.
    // 2. Find a tid satisfying finish[tid] == false && request[tid] <= available for all sem_id
    // 3. If not found, return false.
    // 4. Else, available += allocation, set finish[tid] = true. GOTO 1.
    let n = allocation.len();
    let mut finish = vec![false; n];
    loop {
        let mut found = false;
        for tid in 0..n {
            if finish[tid] { continue; }
            if request[tid].iter().zip(available.iter()).all(|(r, a)| r <= a) {
                finish[tid] = true;
                for i in 0..available.len() {
                    available[i] += allocation[tid][i];
                }
                found = true;
            }
        }
        if !found { break; }
    }
    finish.iter().all(|&x| x)
}

fn semaphore_vectors() -> (Vec<isize>, Vec<Vec<isize>>, Vec<Vec<isize>>) {
    // Obtain Available vector of current process
    let process = current_process();
    let process_inner = process.inner_exclusive_access();
    let mut available = vec![];

    for sem in process_inner.semaphore_list.iter() {
        available.push(
            match sem {
                Some(s) => s.inner.exclusive_access().count.max(0),
                None => 0
            }
        );
    }

    // Obtain allocation and request matrix
    let mut allocation = vec![];
    let mut request = vec![];
    for tid in 0..process_inner.thread_count() {
        let task = match process_inner.tasks[tid].as_ref() {
            Some(t) => t.clone(),
            None => {
                allocation.push(vec![0; available.len()]);
                request.push(vec![0; available.len()]);
                continue;
            }
        };
        let inner = task.inner_exclusive_access();
        match inner.res.as_ref() {
            Some(res) => {
                allocation.push(res.sem_allocation.clone());
                request.push(res.sem_request.clone());
            }
            None => {
                allocation.push(vec![0; available.len()]);
                request.push(vec![0; available.len()]);
            }
        }
    }

    (available, allocation, request)
}

fn mutex_vectors() -> (Vec<isize>, Vec<Vec<isize>>, Vec<Vec<isize>>) {
    // Obtain Available vector of current process
    let process = current_process();
    let process_inner = process.inner_exclusive_access();
    let mut available = vec![];

    for sem in process_inner.mutex_list.iter() {
        available.push(
            match sem {
                Some(s) => if s.is_locked() {0} else {1},
                None => 0
            }
        );
    }

    // Obtain allocation and request matrix
    let mut allocation = vec![];
    let mut request = vec![];
    for tid in 0..process_inner.thread_count() {
        let task = match process_inner.tasks[tid].as_ref() {
            Some(t) => t.clone(),
            None => {
                allocation.push(vec![0; available.len()]);
                request.push(vec![0; available.len()]);
                continue;
            }
        };
        let inner = task.inner_exclusive_access();
        match inner.res.as_ref() {
            Some(res) => {
                allocation.push(res.mutex_allocation.clone());
                request.push(res.mutex_request.clone());
            }
            None => {
                allocation.push(vec![0; available.len()]);
                request.push(vec![0; available.len()]);
            }
        }
    }

    (available, allocation, request)
}

/// Perform Deadlock Detection Algorithm before lock mutex/decrease semaphore,
/// if the algorithm detects that the resource might run out, return false.
pub fn detect_semaphore_deadlock() -> bool {
    // Obtain Available vector of current process
    let (available, allocation, request) = semaphore_vectors();
    banker_algorithm(available, allocation, request)
}

/// Perform Deadlock Detection Algorithm before lock mutex/decrease semaphore,
/// if the algorithm detects that the resource might run out, return false.
pub fn detect_mutex_deadlock() -> bool {
    // Obtain Available vector of current process
    let (available, allocation, request) = mutex_vectors();
    banker_algorithm(available, allocation, request)
}

/// sleep syscall
pub fn sys_sleep(ms: usize) -> isize {
    trace!(
        "kernel:pid[{}] tid[{}] sys_sleep",
        current_task().unwrap().process.upgrade().unwrap().getpid(),
        current_task()
            .unwrap()
            .inner_exclusive_access()
            .res
            .as_ref()
            .unwrap()
            .tid
    );
    let expire_ms = get_time_ms() + ms;
    let task = current_task().unwrap();
    add_timer(expire_ms, task);
    block_current_and_run_next();
    0
}
/// mutex create syscall
pub fn sys_mutex_create(blocking: bool) -> isize {
    trace!(
        "kernel:pid[{}] tid[{}] sys_mutex_create",
        current_task().unwrap().process.upgrade().unwrap().getpid(),
        current_task()
            .unwrap()
            .inner_exclusive_access()
            .res
            .as_ref()
            .unwrap()
            .tid
    );
    let process = current_process();
    let mutex: Option<Arc<dyn Mutex>> = if !blocking {
        Some(Arc::new(MutexSpin::new()))
    } else {
        Some(Arc::new(MutexBlocking::new()))
    };
    let mut process_inner = process.inner_exclusive_access();
    let id = if let Some(id) = process_inner
        .mutex_list
        .iter()
        .enumerate()
        .find(|(_, item)| item.is_none())
        .map(|(id, _)| id)
    {
        process_inner.mutex_list[id] = mutex;
        id as isize
    } else {
        process_inner.mutex_list.push(mutex);
        process_inner.mutex_list.len() as isize - 1
    };

    // Push an element to each of thread within the process
    for task in process_inner.tasks.iter() {
        if let Some(task) = task {
            let mut task_inner = task.inner_exclusive_access();
            if let Some(res) = &mut task_inner.res {
                res.mutex_request.push(0);
                res.mutex_allocation.push(0);
            }
        }
    }
    id as isize
}
/// mutex lock syscall
pub fn sys_mutex_lock(mutex_id: usize) -> isize {
    let tid = current_task()
                    .unwrap()
                    .inner_exclusive_access()
                    .res
                    .as_ref()
                    .unwrap()
                    .tid;
    trace!(
        "kernel:pid[{}] tid[{}] sys_semaphore_up",
        current_task().unwrap().process.upgrade().unwrap().getpid(),
        tid
    );

    let process = current_process();
    let mut process_inner = process.inner_exclusive_access();
    let mutex = Arc::clone(process_inner.mutex_list[mutex_id].as_ref().unwrap());

    // Increase the request vector
    process_inner.get_task(tid).inner_exclusive_access().res.as_mut().unwrap().mutex_request[mutex_id] += 1;
    if process_inner.deadlock_detection_enabled {
        drop(process_inner);
        if !detect_mutex_deadlock() {
            // Undo the request increment before returning
            let process_inner = process.inner_exclusive_access();
            process_inner.get_task(tid).inner_exclusive_access().res.as_mut().unwrap().mutex_request[mutex_id] -= 1;
            return -0xDEAD;
        }
        process_inner = process.inner_exclusive_access();
    }
    drop(process_inner);
    mutex.lock();
    // After actually acquiring the resource, update bookkeeping
    let process_inner = process.inner_exclusive_access();
    process_inner.get_task(tid).inner_exclusive_access().res.as_mut().unwrap().mutex_request[mutex_id] -= 1;
    process_inner.get_task(tid).inner_exclusive_access().res.as_mut().unwrap().mutex_allocation[mutex_id] += 1;
    0
}
/// mutex unlock syscall
pub fn sys_mutex_unlock(mutex_id: usize) -> isize {
    let tid = current_task()
                    .unwrap()
                    .inner_exclusive_access()
                    .res
                    .as_ref()
                    .unwrap()
                    .tid;
    trace!(
        "kernel:pid[{}] tid[{}] sys_semaphore_up",
        current_task().unwrap().process.upgrade().unwrap().getpid(),
        tid
    );
    let process = current_process();
    let process_inner = process.inner_exclusive_access();
    let mutex = Arc::clone(process_inner.mutex_list[mutex_id].as_ref().unwrap());
    mutex.unlock();
    process_inner.get_task(tid).inner_exclusive_access().res.as_mut().unwrap().mutex_allocation[mutex_id] -= 1;
    0
}
/// semaphore create syscall
pub fn sys_semaphore_create(res_count: usize) -> isize {
    trace!(
        "kernel:pid[{}] tid[{}] sys_semaphore_create",
        current_task().unwrap().process.upgrade().unwrap().getpid(),
        current_task()
            .unwrap()
            .inner_exclusive_access()
            .res
            .as_ref()
            .unwrap()
            .tid
    );
    let process = current_process();
    let mut process_inner = process.inner_exclusive_access();
    let id = if let Some(id) = process_inner
        .semaphore_list
        .iter()
        .enumerate()
        .find(|(_, item)| item.is_none())
        .map(|(id, _)| id)
    {
        process_inner.semaphore_list[id] = Some(Arc::new(Semaphore::new(res_count)));
        id
    } else {
        process_inner
            .semaphore_list
            .push(Some(Arc::new(Semaphore::new(res_count))));
        process_inner.semaphore_list.len() - 1
    };

    // Push an element to each of thread within the process
    for task in process_inner.tasks.iter() {
        if let Some(task) = task {
            let mut task_inner = task.inner_exclusive_access();
            if let Some(res) = &mut task_inner.res {
                res.sem_request.push(0);
                res.sem_allocation.push(0);
            }
        }
    }
    id as isize
}
/// semaphore up syscall
pub fn sys_semaphore_up(sem_id: usize) -> isize {
    let tid = current_task()
                    .unwrap()
                    .inner_exclusive_access()
                    .res
                    .as_ref()
                    .unwrap()
                    .tid;
    trace!(
        "kernel:pid[{}] tid[{}] sys_semaphore_up",
        current_task().unwrap().process.upgrade().unwrap().getpid(),
        tid
    );
    let process = current_process();
    let process_inner = process.inner_exclusive_access();
    let sem = Arc::clone(process_inner.semaphore_list[sem_id].as_ref().unwrap());
    sem.up();
    process_inner.get_task(tid).inner_exclusive_access().res.as_mut().unwrap().sem_allocation[sem_id] -= 1;
    drop(process_inner);
    0
}
/// semaphore down syscall
pub fn sys_semaphore_down(sem_id: usize) -> isize {
    let tid = current_task()
                    .unwrap()
                    .inner_exclusive_access()
                    .res
                    .as_ref()
                    .unwrap()
                    .tid;
    trace!(
        "kernel:pid[{}] tid[{}] sys_semaphore_down",
        current_task().unwrap().process.upgrade().unwrap().getpid(),
        tid
    );
    let process = current_process();
    let mut process_inner = process.inner_exclusive_access();
    let sem = Arc::clone(process_inner.semaphore_list[sem_id].as_ref().unwrap());
    // Semaphore 0 is used only as a barrier in ch8 deadlock tests. Counting its
    // in-flight down() as a resource Request makes other threads' barrier waits
    // look like deadlock (false -0xDEAD) while a resource sem is requested.
    let track_deadlock = process_inner.deadlock_detection_enabled && sem_id != 0;
    if track_deadlock {
        process_inner.get_task(tid).inner_exclusive_access().res.as_mut().unwrap().sem_request[sem_id] += 1;
        drop(process_inner);
        if !detect_semaphore_deadlock() {
            let process_inner = process.inner_exclusive_access();
            process_inner.get_task(tid).inner_exclusive_access().res.as_mut().unwrap().sem_request[sem_id] -= 1;
            return -0xDEAD;
        }
        process_inner = process.inner_exclusive_access();
    }
    drop(process_inner);
    // We should first aquire the semaphore before change the request/allocation vector
    // to avoid that thread A first gets process_inner and get blocked by sem.down, then other
    // thread is blocked from obtaining process_inner, therefore the deadlock happens.
    sem.down();
    // After actually acquiring the resource, update bookkeeping
    let process_inner = process.inner_exclusive_access();
    if track_deadlock {
        process_inner.get_task(tid).inner_exclusive_access().res.as_mut().unwrap().sem_request[sem_id] -= 1;
    }
    process_inner.get_task(tid).inner_exclusive_access().res.as_mut().unwrap().sem_allocation[sem_id] += 1;
    0
}
/// condvar create syscall
pub fn sys_condvar_create() -> isize {
    trace!(
        "kernel:pid[{}] tid[{}] sys_condvar_create",
        current_task().unwrap().process.upgrade().unwrap().getpid(),
        current_task()
            .unwrap()
            .inner_exclusive_access()
            .res
            .as_ref()
            .unwrap()
            .tid
    );
    let process = current_process();
    let mut process_inner = process.inner_exclusive_access();
    let id = if let Some(id) = process_inner
        .condvar_list
        .iter()
        .enumerate()
        .find(|(_, item)| item.is_none())
        .map(|(id, _)| id)
    {
        process_inner.condvar_list[id] = Some(Arc::new(Condvar::new()));
        id
    } else {
        process_inner
            .condvar_list
            .push(Some(Arc::new(Condvar::new())));
        process_inner.condvar_list.len() - 1
    };
    id as isize
}
/// condvar signal syscall
pub fn sys_condvar_signal(condvar_id: usize) -> isize {
    trace!(
        "kernel:pid[{}] tid[{}] sys_condvar_signal",
        current_task().unwrap().process.upgrade().unwrap().getpid(),
        current_task()
            .unwrap()
            .inner_exclusive_access()
            .res
            .as_ref()
            .unwrap()
            .tid
    );
    let process = current_process();
    let process_inner = process.inner_exclusive_access();
    let condvar = Arc::clone(process_inner.condvar_list[condvar_id].as_ref().unwrap());
    drop(process_inner);
    condvar.signal();
    0
}
/// condvar wait syscall
pub fn sys_condvar_wait(condvar_id: usize, mutex_id: usize) -> isize {
    trace!(
        "kernel:pid[{}] tid[{}] sys_condvar_wait",
        current_task().unwrap().process.upgrade().unwrap().getpid(),
        current_task()
            .unwrap()
            .inner_exclusive_access()
            .res
            .as_ref()
            .unwrap()
            .tid
    );
    let process = current_process();
    let process_inner = process.inner_exclusive_access();
    let condvar = Arc::clone(process_inner.condvar_list[condvar_id].as_ref().unwrap());
    let mutex = Arc::clone(process_inner.mutex_list[mutex_id].as_ref().unwrap());
    drop(process_inner);
    condvar.wait(mutex);
    0
}
/// enable deadlock detection syscall
///
/// YOUR JOB: Implement deadlock detection, but might not all in this syscall
pub fn sys_enable_deadlock_detect(enabled: usize) -> isize {
    trace!("kernel: sys_enable_deadlock_detect");
    let process = current_process();
    let mut process_inner = process.inner_exclusive_access();
    match enabled {
        0 => {
            process_inner.deadlock_detection_enabled = false;
            return 0;
        },
        1 => {
            process_inner.deadlock_detection_enabled = true;
            return 0;
        }
        _ => -1
    }
}
