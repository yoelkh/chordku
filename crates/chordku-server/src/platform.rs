//! The few operating-system calls the server needs, declared directly against kernel32 so
//! no bindings crate is pulled in. Every item is a no-op on other platforms.

/// Keeps the machine from sleeping while a job runs (a closed lid would otherwise kill a
/// four minute separation). Released on drop.
pub struct KeepAwake;

/// Runs the whole process below normal priority while alive, so a separation that pegs
/// every core does not make the laptop sluggish. Restored on drop.
pub struct BelowNormalPriority;

/// Temporarily back to normal priority (a fast job run in the middle of a separation).
pub struct NormalPriority;

#[cfg(windows)]
mod imp {
    const ES_SYSTEM_REQUIRED: u32 = 0x0000_0001;
    const ES_CONTINUOUS: u32 = 0x8000_0000;
    pub const NORMAL_PRIORITY_CLASS: u32 = 0x0000_0020;
    pub const BELOW_NORMAL_PRIORITY_CLASS: u32 = 0x0000_4000;

    #[repr(C)]
    struct MemoryStatusEx {
        length: u32,
        memory_load: u32,
        total_phys: u64,
        avail_phys: u64,
        total_page_file: u64,
        avail_page_file: u64,
        total_virtual: u64,
        avail_virtual: u64,
        avail_extended_virtual: u64,
    }

    #[link(name = "kernel32")]
    extern "system" {
        fn SetThreadExecutionState(flags: u32) -> u32;
        fn GetCurrentProcess() -> isize;
        fn SetPriorityClass(process: isize, class: u32) -> i32;
        fn GlobalMemoryStatusEx(status: *mut MemoryStatusEx) -> i32;
    }

    pub fn keep_awake(on: bool) {
        let flags = if on { ES_CONTINUOUS | ES_SYSTEM_REQUIRED } else { ES_CONTINUOUS };
        // SAFETY: plain flag-setting Win32 call with no pointers.
        unsafe { SetThreadExecutionState(flags) };
    }

    pub fn set_priority(class: u32) {
        // SAFETY: the pseudo handle from GetCurrentProcess is always valid for this process.
        unsafe { SetPriorityClass(GetCurrentProcess(), class) };
    }

    pub fn free_memory() -> u64 {
        let mut status = MemoryStatusEx {
            length: std::mem::size_of::<MemoryStatusEx>() as u32,
            memory_load: 0,
            total_phys: 0,
            avail_phys: 0,
            total_page_file: 0,
            avail_page_file: 0,
            total_virtual: 0,
            avail_virtual: 0,
            avail_extended_virtual: 0,
        };
        // SAFETY: `status` is a correctly sized, initialised struct owned by this frame.
        if unsafe { GlobalMemoryStatusEx(&mut status) } != 0 {
            status.avail_phys
        } else {
            0
        }
    }
}

#[cfg(not(windows))]
mod imp {
    pub const NORMAL_PRIORITY_CLASS: u32 = 0;
    pub const BELOW_NORMAL_PRIORITY_CLASS: u32 = 0;
    pub fn keep_awake(_on: bool) {}
    pub fn set_priority(_class: u32) {}
    pub fn free_memory() -> u64 {
        0
    }
}

/// Free physical memory in bytes; 0 when unknown.
pub fn free_memory() -> u64 {
    imp::free_memory()
}

impl KeepAwake {
    pub fn new() -> Self {
        imp::keep_awake(true);
        KeepAwake
    }
}

impl Drop for KeepAwake {
    fn drop(&mut self) {
        imp::keep_awake(false);
    }
}

impl BelowNormalPriority {
    pub fn new() -> Self {
        imp::set_priority(imp::BELOW_NORMAL_PRIORITY_CLASS);
        BelowNormalPriority
    }
}

impl Drop for BelowNormalPriority {
    fn drop(&mut self) {
        imp::set_priority(imp::NORMAL_PRIORITY_CLASS);
    }
}

impl NormalPriority {
    pub fn new() -> Self {
        imp::set_priority(imp::NORMAL_PRIORITY_CLASS);
        NormalPriority
    }
}

impl Drop for NormalPriority {
    fn drop(&mut self) {
        imp::set_priority(imp::BELOW_NORMAL_PRIORITY_CLASS);
    }
}
