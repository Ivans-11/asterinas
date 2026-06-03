// SPDX-License-Identifier: MPL-2.0

//! Asterinas-side Axvisor host integration.

#![no_std]
#![deny(unsafe_code)]

mod arch;

extern crate alloc;

use alloc::{
    boxed::Box,
    collections::{BTreeMap, VecDeque},
    sync::Arc,
    vec,
};
use core::{
    str,
    sync::atomic::{AtomicBool, AtomicUsize, Ordering},
};

use axvisor_api::{
    api_impl,
    arch::{self as api_arch, CacheOp},
    console, host, irq,
    memory::{self, PhysAddr, VirtAddr},
    sync, task, time,
};
#[cfg(feature = "shell")]
use ostd::power::ExitCode;
use ostd::{
    cpu::{CpuId, CpuSet, all_cpus},
    mm::{
        Frame, FrameAllocOptions, HasPaddr, HasSize, Infallible, PAGE_SIZE, Segment, Split,
        VmReader, VmWriter, paddr_to_vaddr,
    },
    sync::{LocalIrqDisabled, SpinLock, WaitQueue, Waiter},
    task::Task,
    util::id_set::Id,
};
use spin::Once;

struct HostIfImpl;
struct ConsoleIfImpl;
struct TimeIfImpl;
struct SyncIfImpl;
struct TaskIfImpl;
struct IrqIfImpl;
struct MemoryIfImpl;
struct ArchIfImpl;

#[cfg(target_arch = "riscv64")]
const RISCV_S_EXT_VECTOR: usize = (1usize << (usize::BITS - 1)) + 9;

/// Runtime hook used to spawn proper Asterinas kernel threads for Axvisor.
pub trait KernelTaskRuntime: Sync {
    /// Spawns a kernel thread with the requested CPU affinity.
    fn spawn_task(
        &self,
        entry: Box<dyn FnOnce() + Send + 'static>,
        cpu_affinity: CpuSet,
    ) -> Arc<Task>;
}

static KERNEL_TASK_RUNTIME: Once<&'static dyn KernelTaskRuntime> = Once::new();

static WAIT_QUEUE_IDS: AtomicUsize = AtomicUsize::new(1);
static WAIT_QUEUES: SpinLock<BTreeMap<usize, Arc<WaitQueue>>, LocalIrqDisabled> =
    SpinLock::new(BTreeMap::new());

static TASK_IDS: AtomicUsize = AtomicUsize::new(1);
static TASKS: SpinLock<BTreeMap<usize, Arc<TaskEntry>>, LocalIrqDisabled> =
    SpinLock::new(BTreeMap::new());

static IRQ_HANDLERS: SpinLock<BTreeMap<usize, irq::IrqHandler>, LocalIrqDisabled> =
    SpinLock::new(BTreeMap::new());

static MEMORY_ALLOCS: SpinLock<BTreeMap<usize, HostMemory>, LocalIrqDisabled> =
    SpinLock::new(BTreeMap::new());

static CONSOLE_INPUT: ConsoleInput = ConsoleInput::new();

struct TaskCompletion {
    finished: AtomicBool,
    waiters: WaitQueue,
}

impl TaskCompletion {
    const fn new() -> Self {
        Self {
            finished: AtomicBool::new(false),
            waiters: WaitQueue::new(),
        }
    }

    fn finish(&self) {
        self.finished.store(true, Ordering::Release);
        self.waiters.wake_all();
    }

    fn wait(&self) {
        self.waiters
            .wait_until(|| self.finished.load(Ordering::Acquire).then_some(()));
    }
}

struct TaskEntry {
    task: Arc<Task>,
    completion: Arc<TaskCompletion>,
}

enum HostMemory {
    Frame(Frame<()>),
    Segment(Segment<()>),
}

impl HostMemory {
    fn paddr(&self) -> usize {
        match self {
            Self::Frame(frame) => frame.paddr(),
            Self::Segment(segment) => segment.paddr(),
        }
    }
}

struct ConsoleInput {
    bytes: SpinLock<VecDeque<u8>, LocalIrqDisabled>,
    waiters: WaitQueue,
    registered: AtomicBool,
}

/// Installs the Asterinas kernel-thread runtime used by Axvisor host integration.
pub fn install_kernel_task_runtime(runtime: &'static dyn KernelTaskRuntime) {
    let mut is_new = false;
    KERNEL_TASK_RUNTIME.call_once(|| {
        is_new = true;
        runtime
    });
    assert!(
        is_new,
        "Axvisor kernel task runtime has already been installed"
    );
}

impl ConsoleInput {
    const fn new() -> Self {
        Self {
            bytes: SpinLock::new(VecDeque::new()),
            waiters: WaitQueue::new(),
            registered: AtomicBool::new(false),
        }
    }

    fn ensure_registered(&self) {
        if self
            .registered
            .compare_exchange(false, true, Ordering::AcqRel, Ordering::Acquire)
            .is_err()
        {
            return;
        }

        for (_, device) in aster_console::all_devices() {
            device.register_callback(Box::leak(Box::new(console_input_callback)));
        }
    }
}

fn console_input_callback(mut reader: VmReader<Infallible>) {
    let mut input = vec![0u8; reader.remain()];
    reader.read(&mut VmWriter::from(input.as_mut_slice()));

    if input.is_empty() {
        return;
    }

    let mut bytes = CONSOLE_INPUT.bytes.lock();
    bytes.extend(input);
    drop(bytes);
    CONSOLE_INPUT.waiters.wake_all();
}

fn read_console_bytes(buf: &mut [u8]) -> usize {
    if buf.is_empty() {
        return 0;
    }

    CONSOLE_INPUT.ensure_registered();
    CONSOLE_INPUT.waiters.wait_until(|| {
        let mut bytes = CONSOLE_INPUT.bytes.lock();
        if bytes.is_empty() {
            return None;
        }

        let count = buf.len().min(bytes.len());
        for slot in buf.iter_mut().take(count) {
            *slot = bytes.pop_front().expect("console input buffer underflow");
        }
        Some(count)
    })
}

fn kernel_task_runtime() -> &'static dyn KernelTaskRuntime {
    *KERNEL_TASK_RUNTIME
        .get()
        .expect("Axvisor kernel task runtime is not installed")
}

fn spawn_kernel_task(entry: Box<dyn FnOnce() + Send + 'static>, cpu_affinity: CpuSet) -> Arc<Task> {
    kernel_task_runtime().spawn_task(entry, cpu_affinity)
}

fn cpu_set_from_mask(mask: usize) -> CpuSet {
    let mut cpu_set = CpuSet::new_empty();
    for cpu in all_cpus() {
        let bit = cpu.as_usize();
        if bit < usize::BITS as usize && (mask & (1usize << bit)) != 0 {
            cpu_set.add(cpu);
        }
    }
    assert!(
        !cpu_set.is_empty(),
        "Axvisor requested an empty or invalid host CPU mask: {mask:#x}"
    );
    cpu_set
}

fn get_wait_queue(id: usize) -> Arc<WaitQueue> {
    WAIT_QUEUES
        .lock()
        .get(&id)
        .cloned()
        .expect("wait queue handle not found")
}

fn get_task_entry(handle: task::TaskHandle) -> Arc<TaskEntry> {
    TASKS
        .lock()
        .get(&handle.as_raw())
        .cloned()
        .expect("task handle not found")
}

fn store_host_memory(allocation: HostMemory) -> PhysAddr {
    let paddr = allocation.paddr();
    MEMORY_ALLOCS.lock().insert(paddr, allocation);
    PhysAddr::from_usize(paddr)
}

fn alloc_aligned_segment(num_frames: usize, frame_align: usize) -> Option<Segment<()>> {
    if num_frames == 0 {
        return None;
    }

    let align = frame_align.max(PAGE_SIZE);
    if !align.is_multiple_of(PAGE_SIZE) {
        return None;
    }

    let align_frames = align / PAGE_SIZE;
    let extra_frames = align_frames.saturating_sub(1);
    let total_frames = num_frames.checked_add(extra_frames)?;
    let segment = FrameAllocOptions::new().alloc_segment(total_frames).ok()?;
    let aligned_start = segment.paddr().div_ceil(align) * align;
    let offset = aligned_start - segment.paddr();
    let keep_size = num_frames * PAGE_SIZE;

    if offset == 0 {
        if segment.size() == keep_size {
            return Some(segment);
        }
        let (aligned, suffix) = segment.split(keep_size);
        drop(suffix);
        return Some(aligned);
    }

    let (prefix, tail) = segment.split(offset);
    drop(prefix);

    if tail.size() == keep_size {
        return Some(tail);
    }

    let (aligned, suffix) = tail.split(keep_size);
    drop(suffix);
    Some(aligned)
}

#[api_impl]
impl host::HostIf for HostIfImpl {
    fn prepare_virtualization() {
        arch::prepare_virtualization();
    }

    fn get_host_cpu_num() -> usize {
        ostd::cpu::num_cpus()
    }

    fn spawn_cpu_init_task(cpu_id: usize, task: Box<dyn FnOnce() + Send + 'static>) {
        let cpu = CpuId::try_from(cpu_id).expect("invalid CPU id for Axvisor initialization");
        let _ = spawn_kernel_task(
            Box::new(move || {
                arch::init_percpu();
                task();
            }),
            CpuSet::from(cpu),
        );
    }

    #[cfg(feature = "shell")]
    fn exit(exit_code: i32) -> ! {
        let code = if exit_code == 0 {
            ExitCode::Success
        } else {
            ExitCode::Failure
        };
        ostd::power::poweroff(code)
    }
}

#[api_impl]
impl console::ConsoleIf for ConsoleIfImpl {
    fn write_bytes(bytes: &[u8]) {
        let devices = aster_console::all_devices_lock();
        if devices.is_empty() {
            if let Ok(text) = str::from_utf8(bytes) {
                ostd::early_print!("{}", text);
            }
            return;
        }

        for console in devices.values() {
            console.send(bytes);
        }
    }

    fn read_bytes(bytes: &mut [u8]) -> usize {
        read_console_bytes(bytes)
    }
}

#[api_impl]
impl time::TimeIf for TimeIfImpl {
    fn current_time_nanos() -> time::Nanos {
        aster_time::read_monotonic_time().as_nanos() as u64
    }

    fn set_oneshot_timer(deadline: time::TimeValue) {
        arch::set_oneshot_timer(deadline)
    }
}

#[api_impl]
impl sync::SyncIf for SyncIfImpl {
    fn create_wait_queue() -> usize {
        let id = WAIT_QUEUE_IDS.fetch_add(1, Ordering::Relaxed);
        WAIT_QUEUES.lock().insert(id, Arc::new(WaitQueue::new()));
        id
    }

    fn destroy_wait_queue(queue: usize) {
        WAIT_QUEUES.lock().remove(&queue);
    }

    fn wait_queue_wait(queue: usize) {
        let queue = get_wait_queue(queue);
        let (waiter, _) = Waiter::new_pair();
        queue.enqueue(waiter.waker());
        waiter.wait();
    }

    fn wait_queue_wait_until(queue: usize, condition: Box<dyn Fn() -> bool + Send + 'static>) {
        get_wait_queue(queue).wait_until(|| condition().then_some(()));
    }

    fn wait_queue_wake_one(queue: usize) {
        get_wait_queue(queue).wake_one();
    }

    fn wait_queue_wake_all(queue: usize) {
        get_wait_queue(queue).wake_all();
    }
}

#[api_impl]
impl task::TaskIf for TaskIfImpl {
    fn spawn_task_raw(
        options: task::TaskOptions,
        entry: Box<dyn FnOnce() + Send + 'static>,
    ) -> task::TaskHandle {
        let handle = task::TaskHandle::from_raw(TASK_IDS.fetch_add(1, Ordering::Relaxed));
        let completion = Arc::new(TaskCompletion::new());
        let completion_for_task = completion.clone();
        let registered = Arc::new(AtomicBool::new(false));
        let registered_for_task = registered.clone();
        let cpu_affinity = options
            .cpu_set
            .map_or_else(CpuSet::new_full, cpu_set_from_mask);

        let task = spawn_kernel_task(
            Box::new(move || {
                while !registered_for_task.load(Ordering::Acquire) {
                    Task::yield_now();
                }
                entry();
                completion_for_task.finish();
            }),
            cpu_affinity,
        );

        TASKS
            .lock()
            .insert(handle.as_raw(), Arc::new(TaskEntry { task, completion }));
        registered.store(true, Ordering::Release);
        handle
    }

    fn join_task(task: task::TaskHandle) {
        let entry = get_task_entry(task);
        entry.completion.wait();
        TASKS.lock().remove(&task.as_raw());
    }

    fn current_task() -> Option<task::TaskHandle> {
        let current = Task::current()?.cloned();
        TASKS.lock().iter().find_map(|(&handle, entry)| {
            Arc::ptr_eq(&current, &entry.task).then_some(task::TaskHandle::from_raw(handle))
        })
    }

    fn yield_now() {
        Task::yield_now()
    }
}

#[api_impl]
impl irq::IrqIf for IrqIfImpl {
    fn handle_irq(vector: usize) -> bool {
        #[cfg(target_arch = "riscv64")]
        if vector == RISCV_S_EXT_VECTOR {
            ostd::arch::irq::for_each_pending_external_interrupt(|irq_id| {
                axvisor_core::arch::riscv64::inject_current_interrupt(irq_id);
            });
            return true;
        }

        if let Some(handler) = IRQ_HANDLERS.lock().get(&vector).copied() {
            handler(vector);
            return true;
        }
        false
    }

    fn register_irq_handler(vector: usize, handler: irq::IrqHandler) -> bool {
        let mut handlers = IRQ_HANDLERS.lock();
        if handlers.contains_key(&vector) {
            return false;
        }
        handlers.insert(vector, handler);
        true
    }
}

#[api_impl]
impl memory::MemoryIf for MemoryIfImpl {
    fn alloc_frame() -> Option<PhysAddr> {
        let frame = FrameAllocOptions::new().alloc_frame().ok()?;
        Some(store_host_memory(HostMemory::Frame(frame)))
    }

    fn alloc_contiguous_frames(num_frames: usize, frame_align: usize) -> Option<PhysAddr> {
        let segment = alloc_aligned_segment(num_frames, frame_align)?;
        Some(store_host_memory(HostMemory::Segment(segment)))
    }

    fn dealloc_frame(addr: PhysAddr) {
        let allocation = MEMORY_ALLOCS.lock().remove(&addr.as_usize());
        debug_assert!(matches!(allocation, Some(HostMemory::Frame(_))));
    }

    fn dealloc_contiguous_frames(first_addr: PhysAddr, _num_frames: usize) {
        let allocation = MEMORY_ALLOCS.lock().remove(&first_addr.as_usize());
        debug_assert!(matches!(allocation, Some(HostMemory::Segment(_))));
    }

    fn phys_to_virt(addr: PhysAddr) -> VirtAddr {
        VirtAddr::from_usize(paddr_to_vaddr(addr.as_usize()))
    }

    fn virt_to_phys(addr: VirtAddr) -> PhysAddr {
        arch::linear_mapping_virt_to_phys(addr.as_usize())
    }
}

#[api_impl]
impl api_arch::ArchIf for ArchIfImpl {
    fn dcache_range(op: CacheOp, addr: VirtAddr, size: usize) {
        arch::dcache_range(op, addr, size)
    }

    fn host_fdt_paddr() -> Option<PhysAddr> {
        arch::host_fdt_paddr()
    }

    #[cfg(target_arch = "x86_64")]
    fn host_tsc_frequency_mhz() -> Option<u32> {
        u32::try_from(ostd::arch::tsc_freq() / 1_000_000)
            .ok()
            .filter(|&freq| freq > 0)
    }
}

/// Runs the current Asterinas-side Axvisor integration hook.
pub fn run() {
    aster_logger::print!("[axvisor] starting on Asterinas host runtime\n");
    axvisor_core::boot::run();
}
