// SPDX-License-Identifier: MPL-2.0

//! Asterinas-side Axvisor host integration.

#![no_std]
#![deny(unsafe_code)]

mod arch;

extern crate alloc;

use alloc::{
    boxed::Box,
    collections::{BTreeMap, VecDeque},
    format,
    string::String,
    sync::Arc,
    vec,
    vec::Vec,
};
use core::{
    any::Any,
    str,
    sync::atomic::{AtomicBool, AtomicUsize, Ordering},
};

use ax_errno::{AxResult, ax_err_type};
use axvisor_api::{
    api_impl,
    arch::{self as api_arch, CacheOp},
    console, fs, host, irq,
    memory::{self, PhysAddr, VirtAddr},
    platform, process, task, time, vmm,
};
use ostd::{
    cpu::{CpuId, CpuSet, all_cpus},
    mm::{
        Frame, FrameAllocOptions, HasPaddr, HasSize, Infallible, PAGE_SIZE, Segment, Split,
        VmReader, VmWriter, paddr_to_vaddr,
    },
    power::ExitCode,
    sync::{LocalIrqDisabled, SpinLock, WaitQueue, Waiter},
    task::Task,
    util::id_set::Id,
};
use spin::Once;

struct HostIfImpl;
struct ConsoleIfImpl;
struct TimeIfImpl;
struct PlatformIfImpl;
struct ProcessIfImpl;
struct TaskIfImpl;
struct IrqIfImpl;
struct MemoryIfImpl;
struct VmmIfImpl;
struct ArchIfImpl;
struct FsIfImpl;

const NANOS_PER_SEC: u128 = 1_000_000_000;
#[cfg(target_arch = "riscv64")]
const RISCV_S_EXT_VECTOR: usize = (1usize << (usize::BITS - 1)) + 9;

/// Runtime hook used to spawn proper Asterinas kernel threads for Axvisor.
pub trait KernelTaskRuntime: Sync {
    /// Spawns a kernel thread with the requested CPU affinity and optional task-local data.
    fn spawn_task(
        &self,
        entry: Box<dyn FnOnce() + Send + 'static>,
        cpu_affinity: CpuSet,
        local_data: Option<Box<dyn Any + Send>>,
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

#[derive(Clone, Copy, Debug)]
struct VCpuTaskContext {
    vm_id: vmm::VMId,
    vcpu_id: vmm::VCpuId,
}

impl VCpuTaskContext {
    const fn new(vm_id: vmm::VMId, vcpu_id: vmm::VCpuId) -> Self {
        Self { vm_id, vcpu_id }
    }
}

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
    name: String,
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

#[cfg(any(
    target_arch = "x86_64",
    target_arch = "riscv64",
    target_arch = "loongarch64"
))]
fn host_tick_frequency() -> u64 {
    ostd::arch::tsc_freq()
}

#[cfg(target_arch = "aarch64")]
fn host_tick_frequency() -> u64 {
    1_000_000_000
}

pub(crate) fn host_current_ticks() -> u64 {
    #[cfg(any(
        target_arch = "x86_64",
        target_arch = "riscv64",
        target_arch = "loongarch64"
    ))]
    {
        return ostd::arch::read_tsc();
    }

    #[cfg(target_arch = "aarch64")]
    {
        aster_time::read_monotonic_time().as_nanos() as u64
    }
}

pub(crate) fn host_ticks_to_nanos(ticks: u64) -> u64 {
    let freq = host_tick_frequency() as u128;
    (((ticks as u128) * NANOS_PER_SEC) / freq).min(u64::MAX as u128) as u64
}

pub(crate) fn host_nanos_to_ticks(nanos: u64) -> u64 {
    let freq = host_tick_frequency() as u128;
    (((nanos as u128) * freq) / NANOS_PER_SEC).min(u64::MAX as u128) as u64
}

fn current_vcpu_context() -> VCpuTaskContext {
    let task = Task::current().expect("current VM/vCPU context requested outside of a task");
    *task
        .local_data()
        .downcast_ref::<VCpuTaskContext>()
        .expect("current task is not an Axvisor vCPU task")
}

fn kernel_task_runtime() -> &'static dyn KernelTaskRuntime {
    *KERNEL_TASK_RUNTIME
        .get()
        .expect("Axvisor kernel task runtime is not installed")
}

fn spawn_kernel_task(
    entry: Box<dyn FnOnce() + Send + 'static>,
    cpu_affinity: CpuSet,
    local_data: Option<Box<dyn Any + Send>>,
) -> Arc<Task> {
    kernel_task_runtime().spawn_task(entry, cpu_affinity, local_data)
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
            None,
        );
    }

    fn yield_now() {
        Task::yield_now()
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
    fn current_ticks() -> time::Ticks {
        host_current_ticks()
    }

    fn ticks_to_nanos(ticks: time::Ticks) -> time::Nanos {
        host_ticks_to_nanos(ticks)
    }

    fn nanos_to_ticks(nanos: time::Nanos) -> time::Ticks {
        host_nanos_to_ticks(nanos)
    }

    fn register_timer(
        deadline: time::TimeValue,
        callback: Box<dyn FnOnce(time::TimeValue) + Send + 'static>,
    ) -> time::CancelToken {
        axvisor_core::vmm::timer::register_timer(deadline.as_nanos() as u64, callback)
    }

    fn cancel_timer(token: time::CancelToken) {
        axvisor_core::vmm::timer::cancel_timer(token)
    }

    fn busy_wait(duration: time::TimeValue) {
        let start = host_current_ticks();
        let wait_ticks = host_nanos_to_ticks(duration.as_nanos() as u64);
        while host_current_ticks().wrapping_sub(start) < wait_ticks {
            core::hint::spin_loop();
        }
    }

    fn set_oneshot_timer(deadline: time::TimeValue) {
        arch::set_oneshot_timer(deadline)
    }
}

#[api_impl]
impl platform::PlatformIf for PlatformIfImpl {
    fn get_host_fdt_ptr() -> Option<PhysAddr> {
        arch::get_host_fdt_ptr()
    }

    fn shutdown_host_filesystems() -> AxResult<()> {
        Ok(())
    }
}

#[api_impl]
impl process::ProcessIf for ProcessIfImpl {
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
impl task::TaskIf for TaskIfImpl {
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

    fn wait_queue_wake(queue: usize, count: u32) {
        let queue = get_wait_queue(queue);
        if count == u32::MAX {
            queue.wake_all();
            return;
        }

        for _ in 0..count {
            if !queue.wake_one() {
                break;
            }
        }
    }

    fn spawn_vcpu_task_raw(
        vm_id: vmm::VMId,
        vcpu_id: vmm::VCpuId,
        phys_cpu_set: Option<usize>,
        _stack_size: usize,
        entry: Box<dyn FnOnce() + Send + 'static>,
    ) -> task::TaskHandle {
        let handle = task::TaskHandle::from_raw(TASK_IDS.fetch_add(1, Ordering::Relaxed));
        let completion = Arc::new(TaskCompletion::new());
        let completion_for_task = completion.clone();
        let name = format!("VM[{vm_id}]-VCpu[{vcpu_id}]");
        let cpu_affinity = phys_cpu_set.map_or_else(CpuSet::new_full, cpu_set_from_mask);

        let task = spawn_kernel_task(
            Box::new(move || {
                entry();
                completion_for_task.finish();
            }),
            cpu_affinity,
            Some(Box::new(VCpuTaskContext::new(vm_id, vcpu_id))),
        );

        TASKS.lock().insert(
            handle.as_raw(),
            Arc::new(TaskEntry {
                name,
                task,
                completion,
            }),
        );
        handle
    }

    fn task_id_name(task: task::TaskHandle) -> String {
        get_task_entry(task).name.clone()
    }

    fn task_cpu_id(task: task::TaskHandle) -> usize {
        get_task_entry(task)
            .task
            .schedule_info()
            .cpu
            .get()
            .map_or(0, CpuId::as_usize)
    }

    fn task_join(task: task::TaskHandle) -> i32 {
        let entry = get_task_entry(task);
        entry.completion.wait();
        TASKS.lock().remove(&task.as_raw());
        0
    }
}

#[api_impl]
impl irq::IrqIf for IrqIfImpl {
    fn handle_irq(vector: usize) {
        #[cfg(target_arch = "riscv64")]
        if vector == RISCV_S_EXT_VECTOR {
            ostd::arch::irq::for_each_pending_external_interrupt(|irq_id| {
                axvisor_core::arch::riscv64::inject_interrupt(irq_id);
            });
        }

        if let Some(handler) = IRQ_HANDLERS.lock().get(&vector).copied() {
            handler(vector);
        }
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
impl vmm::VmmIf for VmmIfImpl {
    fn current_vm_id() -> vmm::VMId {
        current_vcpu_context().vm_id
    }

    fn current_vcpu_id() -> vmm::VCpuId {
        current_vcpu_context().vcpu_id
    }

    fn vcpu_num(vm_id: vmm::VMId) -> Option<usize> {
        axvisor_core::vmm::with_vm(vm_id, |vm| vm.vcpu_num())
    }

    fn active_vcpus(vm_id: vmm::VMId) -> Option<usize> {
        Self::vcpu_num(vm_id).map(|vcpu_num| {
            if vcpu_num >= usize::BITS as usize {
                usize::MAX
            } else {
                (1usize << vcpu_num) - 1
            }
        })
    }

    fn inject_interrupt(vm_id: vmm::VMId, vcpu_id: vmm::VCpuId, vector: vmm::InterruptVector) {
        let _ = axvisor_core::vmm::with_vm_and_vcpu_on_pcpu(vm_id, vcpu_id, move |_, vcpu| {
            vcpu.inject_interrupt(vector as usize).unwrap();
        });
    }

    fn inject_interrupt_to_cpus(
        vm_id: vmm::VMId,
        vcpu_set: vmm::VCpuSet,
        vector: vmm::InterruptVector,
    ) {
        for vcpu_id in &vcpu_set {
            Self::inject_interrupt(vm_id, vcpu_id, vector);
        }
    }

    fn notify_vcpu_timer_expired(_vm_id: vmm::VMId, _vcpu_id: vmm::VCpuId) {}
}

#[api_impl]
impl api_arch::ArchIf for ArchIfImpl {
    fn inject_virtual_interrupt(vector: vmm::InterruptVector) {
        arch::inject_virtual_interrupt(vector);
    }

    fn dcache_range(op: CacheOp, addr: VirtAddr, size: usize) {
        arch::dcache_range(op, addr, size)
    }
}

#[api_impl]
impl fs::FsIf for FsIfImpl {
    fn open_file(_path: &str) -> AxResult<usize> {
        Err(ax_err_type!(
            Unsupported,
            "filesystem support is not wired to Asterinas yet"
        ))
    }

    fn create_file(_path: &str) -> AxResult<usize> {
        Err(ax_err_type!(
            Unsupported,
            "filesystem support is not wired to Asterinas yet"
        ))
    }

    fn close_file(_file: usize) {}

    fn file_metadata(_file: usize) -> AxResult<fs::Metadata> {
        Err(ax_err_type!(
            Unsupported,
            "filesystem support is not wired to Asterinas yet"
        ))
    }

    fn file_read(_file: usize, _buf: &mut [u8]) -> AxResult<usize> {
        Err(ax_err_type!(
            Unsupported,
            "filesystem support is not wired to Asterinas yet"
        ))
    }

    fn file_write(_file: usize, _buf: &[u8]) -> AxResult<usize> {
        Err(ax_err_type!(
            Unsupported,
            "filesystem support is not wired to Asterinas yet"
        ))
    }

    fn file_flush(_file: usize) -> AxResult<()> {
        Err(ax_err_type!(
            Unsupported,
            "filesystem support is not wired to Asterinas yet"
        ))
    }

    fn path_metadata(_path: &str) -> AxResult<fs::Metadata> {
        Err(ax_err_type!(
            Unsupported,
            "filesystem support is not wired to Asterinas yet"
        ))
    }

    fn fs_read_dir(_path: &str) -> AxResult<Vec<fs::DirEntry>> {
        Err(ax_err_type!(
            Unsupported,
            "filesystem support is not wired to Asterinas yet"
        ))
    }

    fn fs_create_dir(_path: &str) -> AxResult<()> {
        Err(ax_err_type!(
            Unsupported,
            "filesystem support is not wired to Asterinas yet"
        ))
    }

    fn fs_remove_dir(_path: &str) -> AxResult<()> {
        Err(ax_err_type!(
            Unsupported,
            "filesystem support is not wired to Asterinas yet"
        ))
    }

    fn fs_remove_file(_path: &str) -> AxResult<()> {
        Err(ax_err_type!(
            Unsupported,
            "filesystem support is not wired to Asterinas yet"
        ))
    }

    fn fs_rename(_from: &str, _to: &str) -> AxResult<()> {
        Err(ax_err_type!(
            Unsupported,
            "filesystem support is not wired to Asterinas yet"
        ))
    }

    fn fs_current_dir() -> AxResult<String> {
        Err(ax_err_type!(
            Unsupported,
            "filesystem support is not wired to Asterinas yet"
        ))
    }

    fn fs_set_current_dir(_path: &str) -> AxResult<()> {
        Err(ax_err_type!(
            Unsupported,
            "filesystem support is not wired to Asterinas yet"
        ))
    }
}

/// Runs the current Asterinas-side Axvisor integration hook.
pub fn run() {
    aster_logger::print!("[axvisor] starting on Asterinas host runtime\n");
    axvisor_core::boot::run();
}
