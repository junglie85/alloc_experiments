pub mod mem {
    use std::alloc;
    use std::alloc::{GlobalAlloc, Layout, System};
    use std::cell::UnsafeCell;
    use std::fmt::{write, Display, Formatter};
    use std::ptr::null_mut;
    use std::sync::atomic::{
        AtomicUsize,
        Ordering::{Acquire, SeqCst},
    };

    #[derive(Copy, Clone, Debug)]
    pub enum AllocationContext {
        Arena,
        Pool,
        System,
    }

    #[derive(Debug)]
    struct SystemAllocator {
        allocated: AtomicUsize,
    }

    impl SystemAllocator {
        unsafe fn alloc(&self, layout: Layout) -> *mut u8 {
            let ret = System.alloc(layout);
            if !ret.is_null() {
                self.allocated.fetch_add(layout.size(), SeqCst);
            }
            return ret;
        }

        unsafe fn dealloc(&self, ptr: *mut u8, layout: Layout) {
            System.dealloc(ptr, layout);
            self.allocated.fetch_sub(layout.size(), SeqCst);
        }
    }

    const ARENA_SIZE: usize = 128 * 1024;
    const ARENA_MAX_SUPPORTED_ALIGN: usize = 4096;

    #[derive(Debug)]
    #[repr(C, align(4096))] // 4096 == MAX_SUPPORTED_ALIGN
    struct ArenaAllocator {
        arena: UnsafeCell<[u8; ARENA_SIZE]>,
        remaining: AtomicUsize, // we allocate from the top, counting down
    }

    impl ArenaAllocator {
        unsafe fn alloc(&self, layout: Layout) -> *mut u8 {
            let size = layout.size();
            let align = layout.align();

            // `Layout` contract forbids making a `Layout` with align=0, or align not power of 2.
            // So we can safely use a mask to ensure alignment without worrying about UB.
            let align_mask_to_round_down = !(align - 1);

            if align > ARENA_MAX_SUPPORTED_ALIGN {
                return null_mut();
            }

            let mut allocated = 0;
            if self
                .remaining
                .fetch_update(SeqCst, SeqCst, |mut remaining| {
                    if size > remaining {
                        return None;
                    }
                    remaining -= size;
                    remaining &= align_mask_to_round_down;
                    allocated = remaining;
                    Some(remaining)
                })
                .is_err()
            {
                return null_mut();
            };
            (self.arena.get() as *mut u8).add(allocated)
        }
        unsafe fn dealloc(&self, _ptr: *mut u8, _layout: Layout) {}
    }

    const POOL_SIZE: usize = 128 * 1024;
    const POOL_MAX_SUPPORTED_ALIGN: usize = 4096;

    #[derive(Debug)]
    #[repr(C, align(4096))] // 4096 == MAX_SUPPORTED_ALIGN
    struct PoolAllocator {
        pool: UnsafeCell<[u8; POOL_SIZE]>,
        remaining: AtomicUsize, // we allocate from the top, counting down
    }

    impl PoolAllocator {
        unsafe fn alloc(&self, layout: Layout) -> *mut u8 {
            let size = layout.size();
            let align = layout.align();

            // `Layout` contract forbids making a `Layout` with align=0, or align not power of 2.
            // So we can safely use a mask to ensure alignment without worrying about UB.
            let align_mask_to_round_down = !(align - 1);

            if align > POOL_MAX_SUPPORTED_ALIGN {
                return null_mut();
            }

            let mut allocated = 0;
            if self
                .remaining
                .fetch_update(SeqCst, SeqCst, |mut remaining| {
                    if size > remaining {
                        return None;
                    }
                    remaining -= size;
                    remaining &= align_mask_to_round_down;
                    allocated = remaining;
                    Some(remaining)
                })
                .is_err()
            {
                return null_mut();
            };
            (self.pool.get() as *mut u8).add(allocated)
        }
        unsafe fn dealloc(&self, _ptr: *mut u8, _layout: Layout) {}
    }

    #[derive(Debug)]
    pub struct AllocatorManager {
        ctx: [AllocationContext; 1024],
        ctx_ptr: AtomicUsize,
        system: SystemAllocator,
        arena: ArenaAllocator,
        pool: PoolAllocator,
    }

    #[global_allocator]
    static mut ALLOCATOR: AllocatorManager = AllocatorManager {
        ctx: [AllocationContext::System; 1024],
        ctx_ptr: AtomicUsize::new(0),

        system: SystemAllocator {
            allocated: AtomicUsize::new(0),
        },
        arena: ArenaAllocator {
            arena: UnsafeCell::new([0x55; ARENA_SIZE]),
            remaining: AtomicUsize::new(ARENA_SIZE),
        },
        pool: PoolAllocator {
            pool: UnsafeCell::new([0x55; POOL_SIZE]),
            remaining: AtomicUsize::new(POOL_SIZE),
        },
    };

    unsafe impl Sync for AllocatorManager {}

    unsafe impl GlobalAlloc for AllocatorManager {
        unsafe fn alloc(&self, layout: Layout) -> *mut u8 {
            match self.ctx[self.ctx_ptr.load(Acquire)] {
                AllocationContext::Arena => self.arena.alloc(layout),
                AllocationContext::Pool => self.pool.alloc(layout),
                AllocationContext::System => self.system.alloc(layout),
            }
        }

        unsafe fn dealloc(&self, _ptr: *mut u8, _layout: Layout) {}
    }

    impl AllocatorManager {
        fn push_allocator(&mut self, ctx: AllocationContext) {
            let idx = self.ctx_ptr.fetch_add(1, SeqCst);
            self.ctx[idx + 1] = ctx; // Add 1 because fetch-add returns the previous value.
        }

        fn pop_allocator(&mut self) {
            self.ctx_ptr.fetch_sub(1, SeqCst);
        }

        pub fn info() -> AllocationInfo {
            let system_allocated = unsafe { ALLOCATOR.system.allocated.load(Acquire) };

            let arena_remaining = unsafe { ALLOCATOR.arena.remaining.load(Acquire) };
            let arena_allocated = ARENA_SIZE - arena_remaining;

            let pool_remaining = unsafe { ALLOCATOR.pool.remaining.load(Acquire) };
            let pool_allocated = POOL_SIZE - pool_remaining;

            AllocationInfo {
                system_allocated,
                arena_allocated,
                arena_remaining,
                pool_allocated,
                pool_remaining,
            }
        }
    }

    #[derive(Debug)]
    pub struct AllocationInfo {
        system_allocated: usize,
        arena_allocated: usize,
        arena_remaining: usize,
        pool_allocated: usize,
        pool_remaining: usize,
    }

    pub struct Janitor;

    impl Janitor {
        pub fn new(ctx: AllocationContext) -> Self {
            unsafe { ALLOCATOR.push_allocator(ctx) };

            Self
        }
    }

    impl Drop for Janitor {
        fn drop(&mut self) {
            unsafe { ALLOCATOR.pop_allocator() };
        }
    }
}
